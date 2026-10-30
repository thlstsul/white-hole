#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use white_hole_lib::run;

fn main() {
    run().expect("应用运行失败");
}
