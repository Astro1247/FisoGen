#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod i18n;
mod pricing;
mod storage;
mod ui;

fn main() {
    let result = (|| -> anyhow::Result<()> {
        let store = storage::Store::beside_exe()?;
        let api = api::Api::new()?;
        ui::run(store, api)?;
        Ok(())
    })();
    if let Err(error) = result {
        rfd::MessageDialog::new()
            .set_title("FisoGen")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!("{error:#}"))
            .show();
        std::process::exit(1);
    }
}
