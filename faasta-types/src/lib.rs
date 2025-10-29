pub use stabby;
use stabby::{alloc::string::String as StableString, alloc::vec::Vec as StableVec};

#[stabby::stabby]
#[derive(Clone)]
pub struct Header {
    pub name: StableString,
    pub value: StableString,
}

#[stabby::stabby]
pub struct FaastaRequest {
    pub method: u8,
    pub uri: StableString,
    pub headers: StableVec<Header>,
    pub body: StableVec<u8>,
}

#[stabby::stabby]
pub struct FaastaResponse {
    pub status: u16,
    pub headers: StableVec<Header>,
    pub body: StableVec<u8>,
}

pub type FaastaFuture = stabby::Dyn<
    'static,
    stabby::boxed::Box<()>,
    stabby::vtable!(stabby::future::Future<Output = FaastaResponse> + Send + Sync),
>;

impl FaastaResponse {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: StableVec::new(),
            body: StableVec::new(),
        }
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();
        let name = StableString::from(key.as_str());
        let value = StableString::from(value.as_str());
        self.headers.push(Header { name, value });
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        let bytes: Vec<u8> = body.into();
        self.body = bytes.into_iter().collect();
        self
    }
}

pub mod prelude {
    pub use super::{FaastaFuture, FaastaRequest, FaastaResponse, Header};
    pub use stabby;
}
