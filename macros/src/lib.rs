use proc_macro::{Span, TokenStream};
use quote::{format_ident, quote};
use std::env;
use syn::{FnArg, Ident, ItemFn, LitStr, Pat, ReturnType, Type, TypePath, parse_macro_input};

fn last_path_segment(ty: &Type) -> Option<String> {
    if let Type::Path(TypePath { path, .. }) = ty {
        path.segments
            .last()
            .map(|segment| segment.ident.to_string())
    } else {
        None
    }
}

fn is_faasta_response(ty: &Type) -> bool {
    if let Type::Path(TypePath { path, .. }) = ty {
        if let Some(last) = path.segments.last() {
            if last.ident == "FaastaResponse" {
                return true;
            }
        }
    }
    false
}

#[proc_macro_attribute]
pub fn faasta(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    if input.sig.asyncness.is_none() {
        return syn::Error::new_spanned(&input.sig.ident, "#[faasta] functions must be async")
            .to_compile_error()
            .into();
    }

    let return_ty = match &input.sig.output {
        ReturnType::Type(_, ty) => ty,
        ReturnType::Default => {
            return syn::Error::new_spanned(
                &input.sig.ident,
                "#[faasta] functions must return faasta_types::FaastaResponse",
            )
            .to_compile_error()
            .into();
        }
    };

    if !is_faasta_response(return_ty) {
        return syn::Error::new_spanned(
            return_ty,
            "#[faasta] functions must return faasta_types::FaastaResponse",
        )
        .to_compile_error()
        .into();
    }

    enum ArgKind {
        Request,
        Dir,
    }

    let mut arg_kinds = Vec::new();
    let mut uses_request = false;
    let mut uses_dir = false;

    for arg in &input.sig.inputs {
        match arg {
            FnArg::Receiver(rec) => {
                return syn::Error::new_spanned(rec, "#[faasta] functions must not take self")
                    .to_compile_error()
                    .into();
            }
            FnArg::Typed(pat_type) => {
                let ty = &*pat_type.ty;

                if !matches!(&*pat_type.pat, Pat::Ident(_)) {
                    return syn::Error::new_spanned(
                        &pat_type.pat,
                        "Unsupported argument pattern in #[faasta] function",
                    )
                    .to_compile_error()
                    .into();
                }

                match last_path_segment(ty).as_deref() {
                    Some("FaastaRequest") => {
                        uses_request = true;
                        arg_kinds.push(ArgKind::Request);
                    }
                    Some("Dir") => {
                        uses_dir = true;
                        arg_kinds.push(ArgKind::Dir);
                    }
                    other => {
                        return syn::Error::new_spanned(
                            ty,
                            format!(
                                "Unsupported argument type: {:?}. Supported types are FaastaRequest and Dir",
                                other.unwrap_or("<unknown>")
                            ),
                        )
                        .to_compile_error()
                        .into();
                    }
                }
            }
        }
    }

    let request_ident: Ident = if uses_request {
        format_ident!("__faasta_request")
    } else {
        format_ident!("_faasta_unused_request")
    };
    let dir_ident: Ident = if uses_dir {
        format_ident!("__faasta_dir")
    } else {
        format_ident!("_faasta_unused_dir")
    };

    let original_fn_name = &input.sig.ident;

    let package_name = env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "default_package".to_string());
    let mangled_name = package_name.replace('-', "_");
    let export_fn_name = format_ident!("dy_{}", mangled_name);
    let _export_name_literal = LitStr::new(&export_fn_name.to_string(), Span::call_site().into());

    let call_args: Vec<_> = arg_kinds
        .iter()
        .map(|kind| match kind {
            ArgKind::Request => quote! { #request_ident },
            ArgKind::Dir => quote! { #dir_ident },
        })
        .collect();

    let wrapper = quote! {
        #[unsafe(no_mangle)]
        pub extern "C" fn #export_fn_name(
            #request_ident: faasta_types::FaastaRequest,
            #dir_ident: cap_async_std::fs::Dir
        ) -> faasta_types::FaastaFuture {
            faasta_types::stabby::boxed::Box::new(async move {
                #original_fn_name(#(#call_args),*).await
            }).into()
        }
    };

    let output = quote! {
        #input
        #wrapper
    };

    output.into()
}
