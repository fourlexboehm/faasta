#[derive(Clone, Copy, Debug, Default)]
pub struct Kv;

#[derive(Clone, Debug)]
pub struct Bucket {
    name: String,
}

impl Kv {
    pub fn bucket(&self, name: impl Into<String>) -> Bucket {
        Bucket { name: name.into() }
    }

    pub async fn get(&self, key: &str) -> crate::Result<Option<Vec<u8>>> {
        self.bucket("cache").get(key).await
    }

    pub async fn set(&self, key: &str, value: impl AsRef<[u8]>) -> crate::Result<()> {
        self.bucket("cache").set(key, value).await
    }

    pub async fn delete(&self, key: &str) -> crate::Result<()> {
        self.bucket("cache").delete(key).await
    }
}

impl Bucket {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn get(&self, key: &str) -> crate::Result<Option<Vec<u8>>> {
        get(&self.key(key)).await
    }

    pub async fn set(&self, key: &str, value: impl AsRef<[u8]>) -> crate::Result<()> {
        set(&self.key(key), value.as_ref(), None).await
    }

    pub async fn set_ttl(
        &self,
        key: &str,
        value: impl AsRef<[u8]>,
        ttl_secs: u64,
    ) -> crate::Result<()> {
        set(&self.key(key), value.as_ref(), Some(ttl_secs)).await
    }

    pub async fn delete(&self, key: &str) -> crate::Result<()> {
        delete(&self.key(key)).await
    }

    fn key(&self, key: &str) -> String {
        if self.name == "cache" {
            key.to_string()
        } else {
            format!("{}:{key}", self.name)
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn get(key: &str) -> crate::Result<Option<Vec<u8>>> {
    let bucket = omnia_wasi_keyvalue::cache::open("cache").await?;
    Ok(bucket.get(key).await?)
}

#[cfg(not(target_arch = "wasm32"))]
async fn get(_key: &str) -> crate::Result<Option<Vec<u8>>> {
    anyhow::bail!("faasta::kv is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn set(key: &str, value: &[u8], ttl_secs: Option<u64>) -> crate::Result<()> {
    let bucket = omnia_wasi_keyvalue::cache::open("cache").await?;
    bucket.set(key, value, ttl_secs).await?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn set(_key: &str, _value: &[u8], _ttl_secs: Option<u64>) -> crate::Result<()> {
    anyhow::bail!("faasta::kv is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn delete(key: &str) -> crate::Result<()> {
    let bucket = omnia_wasi_keyvalue::cache::open("cache").await?;
    Ok(bucket.delete(key).await?)
}

#[cfg(not(target_arch = "wasm32"))]
async fn delete(_key: &str) -> crate::Result<()> {
    anyhow::bail!("faasta::kv is only available in a WASI guest")
}
