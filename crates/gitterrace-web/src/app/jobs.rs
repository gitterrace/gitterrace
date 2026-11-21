//! Running page work off the async runtime, under the page job slots and with the
//! request's display context in force.

use std::sync::Arc;

use crate::app::App;
use crate::error::Result;

pub(crate) async fn blocking<T: Send + 'static>(
    app: &Arc<App>,
    f: impl FnOnce(&App) -> Result<T> + Send + 'static,
) -> Result<T> {
    let app = Arc::clone(app);
    let rev = crate::fmt::REQUEST_REV.try_with(Clone::clone).unwrap_or_else(|_| "HEAD".to_owned());
    let switch =
        crate::fmt::REQUEST_SWITCH.try_with(Clone::clone).unwrap_or_else(|_| "/".to_owned());
    // Moved into the job, so a cancelled request cannot free the slot while git still runs.
    let slot = Arc::clone(&app.env.jobs)
        .acquire_owned()
        .await
        .map_err(|_| std::io::Error::other("blocking job slots closed"))?;
    let cfg = app.cfg();
    let absolute = cfg.display.dates == gitterrace_types::DateStyle::Absolute;
    // Every page is built here, so every page reads one table of what is switched on.
    let on = crate::features::Effective::of(&cfg, Some(&app.name));
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        crate::features::with(on, || {
            crate::fmt::with_dates(absolute, || {
                crate::fmt::with_base(&app.base, || {
                    crate::fmt::with_rev(&rev, || crate::fmt::with_switch(&switch, || f(&app)))
                })
            })
        })
    })
    .await?
}
