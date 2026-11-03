// Windows 发行版不弹控制台。注意：release 版 panic 会完全静默，排障请用 debug 构建。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tapper_lib::run()
}
