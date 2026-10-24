//! Android entry point. Put your disc image (any `.iso`/`.gcm`) in the app's files
//! folder: `Android/data/com.gauntletrs.game/files/` on the device's storage.

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use gdl_engine::{app, fs::GameFs};
    // logcat, with the bug reports' event ring beside it.
    gdl_engine::bugreport::install_logger(
        Box::new(android_logger::AndroidLogger::new(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("gauntlet"),
        )),
        log::LevelFilter::Info,
    );
    // Bug reports (debug menu) go where adb can pull them:
    // /sdcard/Android/data/com.gauntletrs.game/files/bugs.
    if let Some(d) = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
    {
        gdl_engine::bugreport::set_dir(d.join("bugs"));
    }
    let dirs: Vec<std::path::PathBuf> = [app.external_data_path(), app.internal_data_path()]
        .into_iter()
        .flatten()
        .collect();
    let fs = dirs.iter().find_map(|d| GameFs::find_in(d));
    let Some(mut fs) = fs else {
        log::error!("no game data found; copy your disc image into {:?}", dirs);
        return;
    };
    // Asset packs: asset-packs/<name>/ folders beside the disc image.
    fs.add_default_packs(&dirs);
    let cfg = app::Config {
        fs,
        level: "levelA1".into(),
        players: Vec::new(),
        mute: false,
        // The memory card lives in the app's private storage.
        save_dir: app.internal_data_path(),
        load_state: None,
    };
    if let Err(e) = app::run_android(cfg, app) {
        log::error!("fatal: {e:#}");
    }
}
