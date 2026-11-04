//! Real application exports must be searchable through the installed command surface.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

#[test]
fn real_office_exports_find_their_text() -> Result<(), Box<dyn Error>> {
    let fixture = support::exclusive_dir("real-office-search")?;
    let documents = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/documents");
    // Original documents exported by LibreOffice 26.8, with an isolated empty profile.
    for name in [
        "short.doc",
        "short.docx",
        "short.odt",
        "short.rtf",
        "short.pdf",
        "workbook.xls",
        "workbook.xlsx",
        "workbook.ods",
        "slides.ppt",
        "slides.pptx",
        "slides.odp",
    ] {
        let root = fixture.join(name.replace('.', "-"));
        fs::create_dir(&root)?;
        fs::write(root.join(name), fs::read(documents.join(name))?)?;
        let output = support::command(BINARY, &fixture)
            .args(["find", "Paris", "-C"])
            .arg(&root)
            .output()?;
        let text = String::from_utf8(output.stdout)?;
        assert!(
            output.status.success(),
            "{name}: {text}, {:?}",
            output.stderr
        );
        assert!(text.contains(name), "{name}: {text}");
        assert!(!text.contains("incomplete"), "{name}: {text}");
    }
    fs::remove_dir_all(fixture)?;
    Ok(())
}
