fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--version")) {
        println!("Alfredo Desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    albert_mission_control::record_native_main_start()
        .expect("Alfredo performance measurement should record native startup");
    albert_mission_control::run();
}
