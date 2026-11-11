//! Measures the per-frame cost of the tree library layout.
//!
//! Usage:
//! cargo run --release -p znicz-tui --example tree_bench -- /path/library.db

use std::path::PathBuf;
use std::time::Instant;

use znicz_library::Library;
use znicz_tui::LibraryLayout;
use znicz_tui::library_pane::LibraryPane;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: tree_bench <library.db>");
    let library = Library::open(&PathBuf::from(path)).expect("open library");
    let mut pane = LibraryPane::new(Some(library));
    pane.set_preferred_layout(LibraryLayout::Tree);

    if pane.artists().is_empty() {
        eprintln!("library has no browse artists");
        return;
    }

    // Expand the first artist, as pressing Enter on the top row does.
    pane.toggle_expand_at_cursor();
    let rows = pane.tree_rows().len();
    println!("tree rows: {rows}");

    let start = Instant::now();
    let mut sink = 0usize;
    for index in 0..rows {
        sink += pane.offset_for(index, 100);
    }
    println!(
        "old per-row offset_for frame: {:?} (sink {sink})",
        start.elapsed()
    );

    let start = Instant::now();
    let selected = pane.selected_index(100);
    let h_offset = pane.h_offset();
    let mut sink = 0usize;
    for index in 0..rows {
        sink += if selected == Some(index) { h_offset } else { 0 };
    }
    println!(
        "single-selection frame: {:?} (sink {sink})",
        start.elapsed()
    );

    let start = Instant::now();
    for _ in 0..3 {
        sink += pane.tree_rows().len();
    }
    println!("3x tree_rows: {:?} (sink {sink})", start.elapsed());
}
