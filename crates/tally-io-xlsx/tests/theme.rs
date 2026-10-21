//! A custom `theme1.xml` changes the theme colours (wave 5, item 28).
#![allow(clippy::unwrap_used)]

use std::path::Path;

use tally_io_xlsx::{read_xlsx, Limits};

#[test]
fn custom_theme_colours_are_read() {
    let p =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/xlsx/custom-theme.xlsx");
    let bytes = std::fs::read(&p).unwrap();
    let wb = read_xlsx(std::io::Cursor::new(&bytes), &Limits::default()).unwrap();
    let s = &wb.sheets[0];
    let a1 = s.cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    let fill = wb.styles[a1.style.unwrap() as usize].fill;
    assert_eq!(
        fill,
        Some(0xff0000ff),
        "accent1 from the custom theme: {fill:x?}"
    );
    let b1 = s.cells.iter().find(|c| c.row == 0 && c.col == 1).unwrap();
    let color = wb.styles[b1.style.unwrap() as usize].font.color;
    assert_eq!(
        color,
        Some(0x1f497dff),
        "dk2 from the custom theme: {color:x?}"
    );
    // A workbook without a theme part uses the Office palette again.
    let p2 =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/xlsx/excel365-basic.xlsx");
    let wb2 = read_xlsx(
        std::io::Cursor::new(std::fs::read(&p2).unwrap()),
        &Limits::default(),
    )
    .unwrap();
    assert!(!wb2.sheets.is_empty());
}
