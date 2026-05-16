use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use tracing::{debug, info, warn};

const SHORTCUT_ID: &str = "toggle";

/// Register the global shortcut with the XDG portal and block until the
/// portal connection closes, forwarding each activation to `tx`.
///
/// Returns an error if the portal is unavailable — caller should log and
/// continue, the daemon is fully functional without a hotkey.
pub async fn register(
    tx: async_channel::Sender<()>,
    parent: Option<&ashpd::WindowIdentifier>,
) -> anyhow::Result<()> {
    let proxy = GlobalShortcuts::new().await?;
    let session = proxy.create_session().await?;

    let request = proxy
        .bind_shortcuts(
            &session,
            &[NewShortcut::new(SHORTCUT_ID, "Show / hide Omniman")
                .preferred_trigger(Some("<Ctrl>space"))],
            parent,
        )
        .await?;

    // response() is sync: prepare_response() was already awaited inside ashpd.
    let bound = request.response()?;
    if bound.shortcuts().is_empty() {
        warn!("GlobalShortcuts portal returned no bound shortcuts");
    } else {
        info!(
            shortcut_id = SHORTCUT_ID,
            "global shortcut registered via XDG portal"
        );
    }

    let mut stream = proxy.receive_activated().await?;
    while let Some(event) = stream.next().await {
        if event.shortcut_id() == SHORTCUT_ID {
            debug!("shortcut activated");
            if tx.send(()).await.is_err() {
                break;
            }
        }
    }
    Ok(())
}
