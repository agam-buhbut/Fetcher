//! systemd service listing + mutation via D-Bus.
//!
//! Talks to `org.freedesktop.systemd1` directly (no fork+exec of `systemctl`)
//! for both system and per-user buses. Reads degrade gracefully when systemd
//! isn't present; mutations surface polkit / permission errors verbatim.

use zbus::blocking::Connection;
use zbus::zvariant::{DynamicType, OwnedObjectPath};
use zbus::Message;

use crate::CoreError;

const SYSTEMD_DEST: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IFACE: &str = "org.freedesktop.systemd1.Manager";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceScope {
    System,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceOp {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

#[derive(Debug, Clone)]
pub struct ServiceUnit {
    pub name: String,
    pub description: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub scope: ServiceScope,
}

fn connect(scope: ServiceScope) -> Result<Connection, CoreError> {
    match scope {
        ServiceScope::System => Connection::system(),
        ServiceScope::User => Connection::session(),
    }
    .map_err(|e| CoreError::DBus(e.to_string()))
}

/// Reply tuple for `ListUnits`. Matches systemd1 Manager interface.
type ListUnitsRow = (
    String,          // name
    String,          // description
    String,          // load state
    String,          // active state
    String,          // sub state
    String,          // followed unit
    OwnedObjectPath, // unit object path
    u32,             // job id
    String,          // job type
    OwnedObjectPath, // job object path
);

/// Issue a method call on the systemd1 Manager interface. Centralizes the
/// destination/path/interface triple so callers only specify the verb + args.
fn call_manager<B>(conn: &Connection, method: &str, body: &B) -> Result<Message, CoreError>
where
    B: serde::ser::Serialize + DynamicType,
{
    conn.call_method(
        Some(SYSTEMD_DEST),
        SYSTEMD_PATH,
        Some(MANAGER_IFACE),
        method,
        body,
    )
    .map_err(|e| map_dbus_err(&e))
}

pub fn list_units(scope: ServiceScope) -> Result<Vec<ServiceUnit>, CoreError> {
    let conn = connect(scope)?;
    let reply: Vec<ListUnitsRow> = call_manager(&conn, "ListUnits", &())?
        .body()
        .deserialize()
        .map_err(|e| CoreError::DBus(e.to_string()))?;

    Ok(reply
        .into_iter()
        .filter(|row| row.0.ends_with(".service"))
        .map(
            |(name, description, load_state, active_state, sub_state, ..)| ServiceUnit {
                name,
                description,
                load_state,
                active_state,
                sub_state,
                scope,
            },
        )
        .collect())
}

pub fn service_action(unit: &str, op: ServiceOp, scope: ServiceScope) -> Result<(), CoreError> {
    let conn = connect(scope)?;
    let unit = unit.to_string();

    match op {
        ServiceOp::Start => {
            call_manager(&conn, "StartUnit", &(unit, "replace".to_string()))?;
        }
        ServiceOp::Stop => {
            call_manager(&conn, "StopUnit", &(unit, "replace".to_string()))?;
        }
        ServiceOp::Restart => {
            call_manager(&conn, "RestartUnit", &(unit, "replace".to_string()))?;
        }
        ServiceOp::Enable => {
            call_manager(&conn, "EnableUnitFiles", &(vec![unit], false, true))?;
            // Reload so systemd picks up the change immediately. Best-effort.
            let _ = call_manager(&conn, "Reload", &());
        }
        ServiceOp::Disable => {
            call_manager(&conn, "DisableUnitFiles", &(vec![unit], false))?;
            let _ = call_manager(&conn, "Reload", &());
        }
    }

    Ok(())
}

fn map_dbus_err(e: &zbus::Error) -> CoreError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("access denied")
        || lower.contains("authentication")
        || lower.contains("interactive authorization")
        || lower.contains("permission")
    {
        CoreError::PermissionDenied(Some(msg))
    } else {
        CoreError::DBus(msg)
    }
}
