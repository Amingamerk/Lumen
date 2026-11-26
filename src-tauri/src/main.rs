// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Context;

fn main() -> anyhow::Result<()> {
    lumen_lib::run().context("Lumen failed to start")
}
