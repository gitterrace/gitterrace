//! The ownership table marks a bus factor of 1 in the warning color, not the error red, leaves
//! 2 and up plain, and never lists an owner whose share rounds to 0%.

use gitterrace_testkit::{Fixture, page};

#[tokio::test]
async fn lone_owners_warn_and_empty_shares_are_left_out() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f = Fixture::init(dir.path());
    // Ann writes almost all of solo/, both share shared/ and Bob touches solo/ once, lightly.
    let big = "x\n".repeat(400);
    f = f.write("solo/a.txt", &big).write("shared/b.txt", "a\n");
    f.git(&["add", "-A"]);
    f.git(&[
        "commit",
        "-q",
        "--author",
        "Ann <ann@x.io>",
        "-m",
        "feat: ann",
    ]);
    f = f
        .write("shared/b.txt", &"b\n".repeat(50))
        .write("solo/c.txt", "c\n");
    f.git(&["add", "-A"]);
    f.git(&[
        "commit",
        "-q",
        "--author",
        "Bob <bob@x.io>",
        "-m",
        "feat: bob",
    ]);
    f = f.write("shared/d.txt", &"d\n".repeat(50));
    f.git(&["add", "-A"]);
    f.git(&[
        "commit",
        "-q",
        "--author",
        "Ann <ann@x.io>",
        "-m",
        "feat: ann again",
    ]);
    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    let (status, body) = page(&router, "/stats/ownership").await;
    assert_eq!(status, 200);
    let table = body.split("class=\"entries own\"").nth(1).expect("table");
    let table = table.split("</table>").next().unwrap_or_default();
    assert!(
        !table.contains("class=\"del\""),
        "bus factor shown as an error"
    );
    for row in table.split("<tr>").skip(2) {
        let bus = row.rsplit("<td class=\"r\">").next().unwrap_or_default();
        if bus.contains(">1</b>") {
            assert!(
                bus.contains("class=\"warn\""),
                "a lone owner without the warning: {row}"
            );
        } else {
            assert!(
                !bus.contains("<b"),
                "a bus factor of 2 or more is marked: {row}"
            );
        }
        assert!(!row.contains(">0%<"), "an owner at 0% is listed: {row}");
    }
}
