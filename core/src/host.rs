//! Synchronous host-service boundary while GUI adapters migrate.
//! Replies belong to the requesting OS thread and exist only during its callback.
use crate::{filesystem, process::Failure};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
type Result<T> = std::result::Result<T, Failure>;
static NEXT: AtomicU64 = AtomicU64::new(1);
thread_local! {static REPLIES: RefCell<BTreeMap<u64,Option<Result<Value>>>> = const {RefCell::new(BTreeMap::new())};}
pub fn reply(id: u64, value: Value) -> Result<Value> {
    respond(id, Ok(value))
}
pub fn fail(id: u64, message: String, cancelled: bool) -> Result<Value> {
    respond(id, Err(Failure { message, cancelled }))
}
fn respond(id: u64, value: Result<Value>) -> Result<Value> {
    REPLIES.with(|replies| {
        let mut replies = replies.borrow_mut();
        let slot = replies
            .get_mut(&id)
            .ok_or("Unknown or expired host request")?;
        if slot.is_some() {
            return Err("Host request already answered".into());
        }
        *slot = Some(value);
        Ok(Value::Null)
    })
}
pub trait Services {
    fn request(&mut self, method: &str, arguments: Value) -> Result<Value>;
    fn poll(&mut self) -> Result<()>;
    fn execution_started(&mut self) -> Result<()> {
        Ok(())
    }
}
pub struct Callback<'a, F: FnMut(Value) -> i32>(pub &'a mut F);
impl<F: FnMut(Value) -> i32> Services for Callback<'_, F> {
    fn execution_started(&mut self) -> Result<()> {
        if (self.0)(json!({"kind":"sync_started"})) != 0 {
            return Err(Failure {
                message: "Sync execution cancelled".into(),
                cancelled: true,
            });
        }
        Ok(())
    }
    fn poll(&mut self) -> Result<()> {
        filesystem::poll(self.0)
    }
    fn request(&mut self, method: &str, arguments: Value) -> Result<Value> {
        self.poll()?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            return Err("Host request identity exhausted".into());
        }
        REPLIES.with(|r| r.borrow_mut().insert(id, None));
        struct Remove(u64);
        impl Drop for Remove {
            fn drop(&mut self) {
                REPLIES.with(|r| r.borrow_mut().remove(&self.0));
            }
        }
        let _guard = Remove(id);
        if (self.0)(json!({"kind":"host_request","id":id,"method":method,"arguments":arguments}))
            != 0
        {
            return Err(Failure {
                message: "Host service request cancelled".into(),
                cancelled: true,
            });
        }
        REPLIES
            .with(|r| r.borrow_mut().get_mut(&id).and_then(Option::take))
            .ok_or_else(|| Failure::from("Host service returned without a reply"))?
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_are_single_use_scoped_and_thread_owned() {
        let mut retained = 0;
        let mut callback = |event: Value| {
            if event["kind"] == "host_request" {
                let id = event["id"].as_u64().unwrap();
                retained = id;
                assert!(std::thread::spawn(move || reply(id, json!("foreign")))
                    .join()
                    .unwrap()
                    .is_err());
                reply(id, json!("answer")).unwrap();
                assert!(reply(id, json!("duplicate")).is_err());
            }
            0
        };
        assert_eq!(
            Callback(&mut callback)
                .request("test", Value::Null)
                .unwrap(),
            "answer"
        );
        assert!(reply(retained, Value::Null).is_err());
    }
    #[test]
    fn missing_reply_and_cancel_release_slots() {
        assert!(Callback(&mut |_| 0).request("test", Value::Null).is_err());
        let mut id = 0;
        assert!(
            Callback(&mut |v: Value| {
                if v["kind"] == "host_request" {
                    id = v["id"].as_u64().unwrap();
                    1
                } else {
                    0
                }
            })
            .request("test", Value::Null)
            .unwrap_err()
            .cancelled
        );
        assert!(reply(id, Value::Null).is_err());
    }
}
