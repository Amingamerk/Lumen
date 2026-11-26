// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tracks;
mod view;

use anyhow::Context;
use tauri::State;
use tracks::{Library, TrackSlice};
use view::View;

const TRACK_COUNT: u32 = 5_000;

#[tauri::command]
fn initial_view(view: State<'_, View>) -> View {
    *view
}

#[tauri::command]
fn get_track_slice(offset: usize, limit: usize, library: State<'_, Library>) -> TrackSlice {
    library.slice(offset, limit)
}

fn main() -> anyhow::Result<()> {
    let args = std::env::args_os().map(|arg| arg.to_string_lossy().into_owned());

    tauri::Builder::default()
        .manage(View::from_args(args))
        .manage(Library::generate(TRACK_COUNT))
        .invoke_handler(tauri::generate_handler![initial_view, get_track_slice])
        .run(tauri::generate_context!())
        .context("failed to run the Tauri app")
}
