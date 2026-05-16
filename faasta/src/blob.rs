#[derive(Clone, Copy, Debug, Default)]
pub struct Blobs;

#[derive(Clone, Debug)]
pub struct Container {
    name: String,
}

impl Blobs {
    pub fn container(&self, name: impl Into<String>) -> Container {
        Container { name: name.into() }
    }
}

impl Container {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn exists(&self) -> crate::Result<bool> {
        container_exists(&self.name).await
    }

    pub async fn create(&self) -> crate::Result<()> {
        create_container(&self.name).await
    }

    pub async fn create_if_missing(&self) -> crate::Result<()> {
        if !self.exists().await? {
            self.create().await?;
        }
        Ok(())
    }

    pub async fn get(&self, name: &str) -> crate::Result<Option<Vec<u8>>> {
        get(&self.name, name).await
    }

    pub async fn put(&self, name: &str, data: impl AsRef<[u8]>) -> crate::Result<()> {
        put(&self.name, name, data.as_ref()).await
    }

    pub async fn list(&self) -> crate::Result<Vec<String>> {
        list(&self.name).await
    }

    pub async fn delete(&self, name: &str) -> crate::Result<()> {
        delete(&self.name, name).await
    }
}

#[cfg(target_arch = "wasm32")]
async fn container_exists(name: &str) -> crate::Result<bool> {
    use anyhow::anyhow;

    omnia_wasi_blobstore::blobstore::container_exists(name.to_string())
        .await
        .map_err(|err| anyhow!("checking container existence: {err}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn container_exists(_name: &str) -> crate::Result<bool> {
    anyhow::bail!("faasta::blob is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn create_container(name: &str) -> crate::Result<()> {
    use anyhow::anyhow;

    let _container = omnia_wasi_blobstore::blobstore::create_container(name.to_string())
        .await
        .map_err(|err| anyhow!("creating container: {err}"))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn create_container(_name: &str) -> crate::Result<()> {
    anyhow::bail!("faasta::blob is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn get(container: &str, name: &str) -> crate::Result<Option<Vec<u8>>> {
    use anyhow::anyhow;
    use omnia_wasi_blobstore::types::IncomingValue;

    let container = omnia_wasi_blobstore::blobstore::get_container(container.to_string())
        .await
        .map_err(|err| anyhow!("opening container: {err}"))?;
    if !container
        .has_object(name.to_string())
        .await
        .map_err(|err| anyhow!("checking object existence: {err}"))?
    {
        return Ok(None);
    }
    let incoming = container
        .get_data(name.to_string(), 0, u64::MAX)
        .await
        .map_err(|err| anyhow!("reading object: {err}"))?;
    let data = IncomingValue::incoming_value_consume_sync(incoming)
        .map_err(|err| anyhow!("consuming incoming value: {err}"))?;
    Ok(Some(data))
}

#[cfg(not(target_arch = "wasm32"))]
async fn get(_container: &str, _name: &str) -> crate::Result<Option<Vec<u8>>> {
    anyhow::bail!("faasta::blob is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn put(container: &str, name: &str, data: &[u8]) -> crate::Result<()> {
    use anyhow::anyhow;
    use omnia_wasi_blobstore::types::OutgoingValue;

    let container = omnia_wasi_blobstore::blobstore::get_container(container.to_string())
        .await
        .map_err(|err| anyhow!("opening container: {err}"))?;
    let outgoing = OutgoingValue::new_outgoing_value();
    {
        let body = outgoing
            .outgoing_value_write_body()
            .await
            .map_err(|err| anyhow!("getting write body: {err:?}"))?;
        body.blocking_write_and_flush(data)
            .map_err(|err| anyhow!("writing data: {err}"))?;
    };
    container
        .write_data(name.to_string(), &outgoing)
        .await
        .map_err(|err| anyhow!("writing object: {err}"))?;
    OutgoingValue::finish(outgoing).map_err(|err| anyhow!("finishing write: {err}"))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn put(_container: &str, _name: &str, _data: &[u8]) -> crate::Result<()> {
    anyhow::bail!("faasta::blob is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn list(container: &str) -> crate::Result<Vec<String>> {
    use anyhow::anyhow;

    let container = omnia_wasi_blobstore::blobstore::get_container(container.to_string())
        .await
        .map_err(|err| anyhow!("opening container: {err}"))?;
    let stream = container
        .list_objects()
        .await
        .map_err(|err| anyhow!("listing objects: {err}"))?;
    let mut names = Vec::new();
    loop {
        let (batch, done) = stream
            .read_stream_object_names(100)
            .await
            .map_err(|err| anyhow!("reading object names: {err}"))?;
        names.extend(batch);
        if done {
            break;
        }
    }
    Ok(names)
}

#[cfg(not(target_arch = "wasm32"))]
async fn list(_container: &str) -> crate::Result<Vec<String>> {
    anyhow::bail!("faasta::blob is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn delete(container: &str, name: &str) -> crate::Result<()> {
    use anyhow::anyhow;

    let container = omnia_wasi_blobstore::blobstore::get_container(container.to_string())
        .await
        .map_err(|err| anyhow!("opening container: {err}"))?;
    container
        .delete_object(name.to_string())
        .await
        .map_err(|err| anyhow!("deleting object: {err}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn delete(_container: &str, _name: &str) -> crate::Result<()> {
    anyhow::bail!("faasta::blob is only available in a WASI guest")
}
