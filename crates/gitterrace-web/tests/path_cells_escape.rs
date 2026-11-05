//! A path with markup in it renders as text in every path cell. Paths are data the
//! shared path macro escapes, so a directory like `<b>&c` never becomes a tag in the hotspots,
//! coupled files or ownership tables.

use gitterrace_testkit::{Fixture, oracle, page};

const RAW: &str = "<b>&c";
const SAFE: &str = "<span class=\"seg\">&#60;b&#62;&#38;c</span>";

#[test]
fn a_marked_up_file_name_is_text_in_every_path_table() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let mut f = Fixture::init(dir.path());
        // Three commits touching both files, so they rank as hot and as coupled.
        for (i, day) in ["2024-03-01", "2024-03-02", "2024-03-03"]
            .iter()
            .enumerate()
        {
            f = f
                .at(&format!("{day}T10:00:00Z"))
                .write(&format!("{RAW}/lib.rs"), &format!("{i}\n"))
                .write("src/other.rs", &format!("{i}\n"))
                .commit("feat(core): change both");
        }
        let router = gitterrace_web::app_router(f.path()).expect("router");
        oracle::ready(&router).await;

        for (uri, section) in [
            ("/stats/hotspots?period=all", "hotspots"),
            ("/stats/hotspots?period=all", "coupled"),
            ("/stats/ownership?period=all", "ownership"),
        ] {
            let (status, html) = page(&router, uri).await;
            assert_eq!(status, 200, "{uri}");
            assert!(!html.contains(RAW), "{section}: raw name in {uri}");
            assert!(
                html.contains(SAFE),
                "{section}: escaped name missing in {uri}"
            );
        }
    });
}
