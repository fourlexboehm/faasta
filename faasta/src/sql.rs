pub use omnia_wasi_sql::{DataType, Field, Row};

#[derive(Clone, Copy, Debug, Default)]
pub struct Sql;

pub trait IntoSqlParams {
    fn into_sql_params(self) -> Vec<DataType>;
}

pub trait IntoSqlValue {
    fn into_sql_value(self) -> DataType;
}

impl Sql {
    pub async fn exec(
        &self,
        query: impl Into<String>,
        params: impl IntoSqlParams,
    ) -> crate::Result<u32> {
        exec(query.into(), params.into_sql_params()).await
    }

    pub async fn query(
        &self,
        statement: impl Into<String>,
        params: impl IntoSqlParams,
    ) -> crate::Result<Vec<Row>> {
        query(statement.into(), params.into_sql_params()).await
    }
}

#[cfg(target_arch = "wasm32")]
async fn query(query: String, params: Vec<DataType>) -> crate::Result<Vec<Row>> {
    use anyhow::anyhow;
    use omnia_wasi_sql::types::{Connection, Statement};

    let connection = Connection::open("default".to_string())
        .await
        .map_err(|err| anyhow!("failed to open SQL connection: {}", err.trace()))?;
    let statement = Statement::prepare(query, params)
        .await
        .map_err(|err| anyhow!("failed to prepare SQL statement: {}", err.trace()))?;
    let rows = omnia_wasi_sql::readwrite::query(&connection, &statement)
        .await
        .map_err(|err| anyhow!("SQL query failed: {}", err.trace()))?;
    Ok(rows)
}

#[cfg(not(target_arch = "wasm32"))]
async fn query(_query: String, _params: Vec<DataType>) -> crate::Result<Vec<Row>> {
    anyhow::bail!("faasta::sql is only available in a WASI guest")
}

#[cfg(target_arch = "wasm32")]
async fn exec(query: String, params: Vec<DataType>) -> crate::Result<u32> {
    use anyhow::anyhow;
    use omnia_wasi_sql::types::{Connection, Statement};

    let connection = Connection::open("default".to_string())
        .await
        .map_err(|err| anyhow!("failed to open SQL connection: {}", err.trace()))?;
    let statement = Statement::prepare(query, params)
        .await
        .map_err(|err| anyhow!("failed to prepare SQL statement: {}", err.trace()))?;
    let count = omnia_wasi_sql::readwrite::exec(&connection, &statement)
        .await
        .map_err(|err| anyhow!("SQL exec failed: {}", err.trace()))?;
    Ok(count)
}

#[cfg(not(target_arch = "wasm32"))]
async fn exec(_query: String, _params: Vec<DataType>) -> crate::Result<u32> {
    anyhow::bail!("faasta::sql is only available in a WASI guest")
}

impl IntoSqlParams for () {
    fn into_sql_params(self) -> Vec<DataType> {
        Vec::new()
    }
}

impl IntoSqlParams for Vec<DataType> {
    fn into_sql_params(self) -> Vec<DataType> {
        self
    }
}

impl<T> IntoSqlParams for (T,)
where
    T: IntoSqlValue,
{
    fn into_sql_params(self) -> Vec<DataType> {
        vec![self.0.into_sql_value()]
    }
}

impl IntoSqlValue for String {
    fn into_sql_value(self) -> DataType {
        DataType::Str(Some(self))
    }
}

impl IntoSqlValue for &str {
    fn into_sql_value(self) -> DataType {
        DataType::Str(Some(self.to_string()))
    }
}

impl IntoSqlValue for bool {
    fn into_sql_value(self) -> DataType {
        DataType::Boolean(Some(self))
    }
}

impl IntoSqlValue for i32 {
    fn into_sql_value(self) -> DataType {
        DataType::Int32(Some(self))
    }
}

impl IntoSqlValue for i64 {
    fn into_sql_value(self) -> DataType {
        DataType::Int64(Some(self))
    }
}

impl IntoSqlValue for u32 {
    fn into_sql_value(self) -> DataType {
        DataType::Uint32(Some(self))
    }
}

impl IntoSqlValue for u64 {
    fn into_sql_value(self) -> DataType {
        DataType::Uint64(Some(self))
    }
}

impl IntoSqlValue for f32 {
    fn into_sql_value(self) -> DataType {
        DataType::Float(Some(self))
    }
}

impl IntoSqlValue for f64 {
    fn into_sql_value(self) -> DataType {
        DataType::Double(Some(self))
    }
}

impl IntoSqlValue for Vec<u8> {
    fn into_sql_value(self) -> DataType {
        DataType::Binary(Some(self))
    }
}
