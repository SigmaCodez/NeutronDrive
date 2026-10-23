//! Desktop portal transactions and appearance signals owned by Rust.
use crate::{desktop_bus::bounded, process::Failure};
use futures_core::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::{poll_fn, Future},
    io::Read,
    pin::Pin,
    task::Poll,
    time::{Duration, Instant},
};
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";
type Result<T> = std::result::Result<T, Failure>;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Background { enabled: bool },
    Appearance,
    OpenUri { uri: String },
}
async fn next(stream: &mut zbus::MessageStream) -> Option<zbus::Result<zbus::Message>> {
    poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}
fn background_accepted(
    code: u32,
    values: &HashMap<String, zbus::OwnedVariant>,
    enabled: bool,
) -> bool {
    let flag = |name| {
        values
            .get(name)
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false)
    };
    code == 0 && (!enabled || flag("background")) && flag("autostart") == enabled
}
async fn background(enabled: bool) -> zbus::Result<bool> {
    let connection = zbus::Connection::session().await?;
    let mut random = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token = format!(
        "neutrondrive_{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    // Subscribe before calling, including changed request paths and early responses.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(PORTAL)
        .interface("org.freedesktop.portal.Request")
        .member("Response")
        .path_namespace("/org/freedesktop/portal/desktop/request")
        .build()?;
    let mut stream =
        zbus::MessageStream::for_match_rule(rule, &connection, std::num::NonZeroUsize::new(64))
            .await?;
    let options = HashMap::from([
        ("handle_token", zbus::Variant::new(token.as_str())),
        (
            "reason",
            zbus::Variant::new("Ausgewählte Proton-Drive-Ordner automatisch synchronisieren"),
        ),
        ("autostart", zbus::Variant::new(enabled)),
        (
            "commandline",
            zbus::Variant::new(vec!["neutrondrive", "--background"]),
        ),
        ("dbus-activatable", zbus::Variant::new(false)),
    ]);
    let reply = connection
        .call_method(
            Some(PORTAL),
            PATH,
            Some("org.freedesktop.portal.Background"),
            "RequestBackground",
            &("", options),
        )
        .await?;
    let owner = reply.header().sender().map(|v| v.as_str().to_owned());
    if owner.is_none() {
        return Ok(false);
    }
    let path: zbus::OwnedObjectPath = reply.body().deserialize()?;
    if !path
        .as_str()
        .starts_with("/org/freedesktop/portal/desktop/request/")
    {
        return Ok(false);
    }
    while let Some(message) = next(&mut stream).await {
        let message = message?;
        if message.header().sender().map(|v| v.as_str()) != owner.as_deref() {
            continue;
        }
        if message.header().path().map(|v| v.as_str()) != Some(path.as_str()) {
            continue;
        }
        let (code, values): (u32, HashMap<String, zbus::OwnedVariant>) =
            message.body().deserialize()?;
        return Ok(background_accepted(code, &values, enabled));
    }
    Ok(false)
}
fn browser_uri(uri: &str) -> bool {
    if uri.len() > 16384
        || uri
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let resource = uri.split('#').next().unwrap_or("");
    resource.parse::<ureq::http::Uri>().is_ok_and(|value| {
        matches!(value.scheme_str(), Some("http" | "https"))
            && value.host().is_some_and(|host| !host.is_empty())
    })
}
async fn open_uri(uri: &str) -> zbus::Result<bool> {
    let connection = zbus::Connection::session().await?;
    let mut random = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token = format!(
        "neutrondrive_{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(PORTAL)
        .interface("org.freedesktop.portal.Request")
        .member("Response")
        .path_namespace("/org/freedesktop/portal/desktop/request")
        .build()?;
    let mut stream =
        zbus::MessageStream::for_match_rule(rule, &connection, std::num::NonZeroUsize::new(64))
            .await?;
    let options = HashMap::from([("handle_token", zbus::Variant::new(token.as_str()))]);
    let reply = connection
        .call_method(
            Some(PORTAL),
            PATH,
            Some("org.freedesktop.portal.OpenURI"),
            "OpenURI",
            &("", uri, options),
        )
        .await?;
    let owner = reply.header().sender().map(|v| v.as_str().to_owned());
    let path: zbus::OwnedObjectPath = reply.body().deserialize()?;
    if owner.is_none()
        || !path
            .as_str()
            .starts_with("/org/freedesktop/portal/desktop/request/")
    {
        return Ok(false);
    }
    while let Some(message) = next(&mut stream).await {
        let message = message?;
        if message.header().sender().map(|v| v.as_str()) != owner.as_deref()
            || message.header().path().map(|v| v.as_str()) != Some(path.as_str())
        {
            continue;
        }
        let (code, _): (u32, HashMap<String, zbus::OwnedVariant>) = message.body().deserialize()?;
        return Ok(code == 0);
    }
    Ok(false)
}
fn scheme(value: &zbus::OwnedVariant) -> Option<u32> {
    // Settings.Read predates ReadOne and wraps the setting in two variants.
    let value = match &**value {
        zbus::Variant::Variant(inner) => &**inner,
        value => value,
    };
    match value {
        zbus::Variant::U32(v) => Some(if *v <= 2 { *v } else { 0 }),
        _ => None,
    }
}
fn changed(message: &zbus::Message) -> Option<u32> {
    let (namespace, key, value): (String, String, zbus::OwnedVariant) =
        message.body().deserialize().ok()?;
    if namespace != APPEARANCE || key != "color-scheme" {
        return None;
    }
    scheme(&value)
}
fn emit(scheme: u32, callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
    if callback(json!({"kind":"appearance_changed","scheme":scheme})) != 0 {
        return Err(Failure {
            message: "Darstellungsüberwachung beendet.".into(),
            cancelled: true,
        });
    }
    Ok(())
}
async fn appearance(callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    let connection = bounded(
        zbus::Connection::session(),
        Duration::from_secs(5),
        callback,
    )
    .await?
    .map_err(|_| "Desktop-Portal ist nicht verfügbar.")?;
    let proxy = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&connection)
        .destination(PORTAL)
        .path(PATH)
        .interface("org.freedesktop.portal.Settings")
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|_| "Ungültige Portal-Schnittstelle.")?;
    let mut stream = bounded(
        proxy.receive_signal_with_args("SettingChanged", &[(0, APPEARANCE)]),
        Duration::from_secs(3),
        callback,
    )
    .await?
    .map_err(|_| "Desktop-Portal ist nicht verfügbar.")?;
    let args = (APPEARANCE, "color-scheme");
    let mut read = Box::pin(connection.call_method(
        Some(PORTAL),
        PATH,
        Some("org.freedesktop.portal.Settings"),
        "Read",
        &args,
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = false;
    enum Initial {
        Read(zbus::Result<zbus::Message>),
        Signal(Option<zbus::Message>),
    }
    loop {
        let select = poll_fn(|cx| {
            if let Poll::Ready(message) = Pin::new(&mut stream).poll_next(cx) {
                return Poll::Ready(Initial::Signal(message));
            }
            read.as_mut().poll(cx).map(Initial::Read)
        });
        match bounded(
            select,
            deadline.saturating_duration_since(Instant::now()),
            callback,
        )
        .await
        {
            Ok(Initial::Signal(Some(message))) => {
                if let Some(value) = changed(&message) {
                    seen = true;
                    emit(value, callback)?;
                }
            }
            Ok(Initial::Signal(_)) => return Err("Desktop-Portal wurde getrennt.".into()),
            Ok(Initial::Read(result)) => {
                if !seen {
                    if let Ok(message) = result {
                        if let Ok(value) = message.body().deserialize::<zbus::OwnedVariant>() {
                            if let Some(value) = scheme(&value) {
                                emit(value, callback)?;
                            }
                        }
                    }
                }
                break;
            }
            Err(e) if e.cancelled => return Err(e),
            Err(_) => break,
        }
    }
    drop(read);
    loop {
        let message = bounded(
            poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)),
            Duration::from_secs(365 * 86400),
            callback,
        )
        .await?
        .ok_or("Desktop-Portal wurde getrennt.")?;
        if let Some(value) = changed(&message) {
            emit(value, callback)?;
        }
    }
}
pub fn run(operation: Operation, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    zbus::block_on(async {
        match operation {
            Operation::OpenUri { uri } => {
                if !browser_uri(&uri) {
                    return Err("Only valid HTTP(S) browser URLs are supported".into());
                }
                match bounded(open_uri(&uri), Duration::from_secs(120), callback).await? {
                    Ok(true) => Ok(json!(true)),
                    _ => Err("The desktop portal did not open the browser".into()),
                }
            }
            Operation::Background { enabled } => {
                match bounded(background(enabled), Duration::from_secs(120), callback).await {
                    Ok(Ok(value)) => Ok(json!(value)),
                    Err(e) if e.cancelled => Err(e),
                    _ => Ok(json!(false)),
                }
            }
            Operation::Appearance => appearance(callback).await,
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_links_reject_non_web_schemes_and_ambiguous_authorities() {
        for uri in [
            "https://account.proton.me/auth-desktop?test=one#fragment",
            "http://localhost:8080/path",
        ] {
            assert!(browser_uri(uri), "{uri}");
        }
        for uri in [
            "file:///tmp/test",
            "javascript:alert(1)",
            "https:/missing",
            "https://",
            "https://example.test\\evil",
            "https://example.test/\n",
            "https://example.test/a b",
        ] {
            assert!(!browser_uri(uri), "{uri}");
        }
    }
    #[test]
    fn background_response_requires_matching_consent() {
        let values = HashMap::from([
            ("background".into(), true.into()),
            ("autostart".into(), true.into()),
        ]);
        assert!(background_accepted(0, &values, true));
        assert!(!background_accepted(1, &values, true));
        assert!(!background_accepted(0, &values, false));
        let values = HashMap::from([("autostart".into(), false.into())]);
        assert!(background_accepted(0, &values, false));
        assert!(!background_accepted(0, &values, true));
        let values = HashMap::from([
            ("autostart".into(), 1u32.into()),
            ("background".into(), true.into()),
        ]);
        assert!(!background_accepted(0, &values, true));
    }
    #[test]
    fn only_valid_unsigned_portal_schemes_are_accepted() {
        for i in 0..=2 {
            assert_eq!(scheme(&zbus::OwnedVariant::from(i)), Some(i));
        }
        assert_eq!(scheme(&zbus::OwnedVariant::from(3u32)), Some(0));
        assert_eq!(scheme(&zbus::OwnedVariant::from(true)), None);
    }
}
