use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Pat, Type, TypePath, parse_macro_input};

fn last_path_segment(ty: &Type) -> Option<String> {
    if let Type::Path(TypePath { path, .. }) = ty {
        path.segments
            .last()
            .map(|segment| segment.ident.to_string())
    } else {
        None
    }
}

#[proc_macro_attribute]
pub fn handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    if input.sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            &input.sig.ident,
            "#[faasta::handler] functions must be async",
        )
        .to_compile_error()
        .into();
    }

    enum ArgKind {
        Kv,
        Sql,
        Blobs,
    }

    let mut arg_kinds = Vec::new();

    for arg in &input.sig.inputs {
        match arg {
            FnArg::Receiver(rec) => {
                return syn::Error::new_spanned(
                    rec,
                    "#[faasta::handler] functions must not take self",
                )
                .to_compile_error()
                .into();
            }
            FnArg::Typed(pat_type) => {
                if !matches!(&*pat_type.pat, Pat::Ident(_)) {
                    return syn::Error::new_spanned(
                        &pat_type.pat,
                        "unsupported argument pattern in #[faasta::handler] function",
                    )
                    .to_compile_error()
                    .into();
                }

                match last_path_segment(&pat_type.ty).as_deref() {
                    Some("Kv") => arg_kinds.push(ArgKind::Kv),
                    Some("Sql") => arg_kinds.push(ArgKind::Sql),
                    Some("Blobs") => arg_kinds.push(ArgKind::Blobs),
                    other => {
                        return syn::Error::new_spanned(
                            &pat_type.ty,
                            format!(
                                "unsupported argument type: {:?}. Supported injected types are Kv, Sql, and Blobs",
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

    let original_fn_name = &input.sig.ident;
    let export_type = format_ident!("__Faasta{}Handler", original_fn_name);
    let call_args: Vec<_> = arg_kinds
        .iter()
        .map(|kind| match kind {
            ArgKind::Kv => quote! { ::faasta::kv::Kv::default() },
            ArgKind::Sql => quote! { ::faasta::sql::Sql::default() },
            ArgKind::Blobs => quote! { ::faasta::blob::Blobs::default() },
        })
        .collect();

    let output = quote! {
        #input

        struct #export_type;

        impl ::faasta::__private::wasip3::exports::http::handler::Guest for #export_type {
            async fn handle(
                _request: ::faasta::__private::wasip3::http::types::Request,
            ) -> ::core::result::Result<
                ::faasta::__private::wasip3::http::types::Response,
                ::faasta::__private::wasip3::http::types::ErrorCode,
            > {
                ::faasta::__private::response_from_result(
                    #original_fn_name(#(#call_args),*).await
                )
            }
        }

        ::faasta::__private::wasip3::http::service::export!(#export_type);
    };

    output.into()
}
