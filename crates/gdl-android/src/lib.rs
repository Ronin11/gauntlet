//! Android entry point. Put your disc image (any `.iso`/`.gcm`) in the app's files
//! folder: `Android/data/com.gauntletrs.game/files/` on the device's storage.

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use gdl_engine::{app, fs::GameFs};
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("gauntlet"),
    );
    let dirs: Vec<std::path::PathBuf> = [app.external_data_path(), app.internal_data_path()]
        .into_iter()
        .flatten()
        .collect();
    let fs = dirs.iter().find_map(|d| GameFs::find_in(d));
    let Some(fs) = fs else {
        log::error!("no game data found; copy your disc image into {:?}", dirs);
        return;
    };
    let cfg = app::Config {
        fs,
        level: "levelA1".into(),
        players: Vec::new(),
        mute: false,
        // The memory card lives in the app's private storage.
        save_dir: app.internal_data_path(),
    };
    if let Err(e) = app::run_android(cfg, app) {
        log::error!("fatal: {e:#}");
    }
}
