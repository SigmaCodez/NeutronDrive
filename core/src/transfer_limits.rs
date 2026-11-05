//! Private transfer settings passed to each serialized SDK command.
use crate::{
    process::{self, Failure},
    storage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub upload_kib: u32,
    pub download_kib: u32,
}
impl Limits {
    pub fn validate(&self) -> Result<(), Failure> {
        if self.upload_kib > 1048576 || self.download_kib > 1048576 {
            return Err("Transfer limits must be between 0 and 1048576 KiB/s".into());
        }
        Ok(())
    }
    pub fn enabled(&self) -> bool {
        self.upload_kib != 0 || self.download_kib != 0
    }
}
pub fn read() -> Result<Limits, Failure> {
    let value = storage::read(
        &process::data_dir()?.join("transfer-limits.json"),
        json!({"upload_kib":0,"download_kib":0}),
        4096,
        true,
    )?;
    let limits: Limits = serde_json::from_value(value)
        .map_err(|_| Failure::from("Invalid saved transfer limits"))?;
    limits.validate()?;
    Ok(limits)
}
pub fn run(value: Option<Limits>) -> Result<Value, Failure> {
    if let Some(limits) = value {
        limits.validate()?;
        storage::write(
            &process::data_dir()?.join("transfer-limits.json"),
            json!(limits),
        )?;
        return Ok(json!(limits));
    }
    Ok(json!(read()?))
}
