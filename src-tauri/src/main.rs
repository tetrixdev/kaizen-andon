// The release build must not open a console window behind the widget.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    kaizen_andon_lib::run()
}
