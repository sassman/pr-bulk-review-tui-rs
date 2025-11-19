// We'll use the parser indirectly through parse_workflow_logs
use gh_actions_log_parser::parse_workflow_logs;
use std::io::Write;

#[test]
fn test_256_color_parsing() {
    // Create a minimal log with the problematic ANSI sequence
    // This is the actual ANSI sequence from line 416 of the fixture
    let log_content = "2025-11-15T19:57:15.2062090Z \x1b[0m\x1b[1m\x1b[38;5;9merror[E0425]\x1b[0m\x1b[0m\x1b[1m: cannot find value\x1b[0m\n";

    // Create a ZIP with this content
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::FileOptions::<()>::default();
    zip.start_file("test.txt", options).unwrap();
    zip.write_all(log_content.as_bytes()).unwrap();
    let zip_data = zip.finish().unwrap().into_inner();

    // Parse it
    let parsed = parse_workflow_logs(&zip_data).unwrap();
    let job_log = &parsed.jobs[0];

    // Find the line with the error
    let error_line = job_log
        .lines
        .iter()
        .find(|line| line.display_content.contains("error[E0425]"))
        .expect("Should find error line");

    let segments = &error_line.styled_segments;

    println!("\nParsed segments:");
    for (i, seg) in segments.iter().enumerate() {
        println!("  Segment {}: '{}'", i, seg.text);
        println!("    bold: {}", seg.style.bold);
        println!("    strikethrough: {}", seg.style.strikethrough);
        println!("    fg_color: {:?}", seg.style.fg_color);
    }

    // The "error[E0425]" text should NOT have strikethrough
    let error_segment = segments
        .iter()
        .find(|s| s.text.contains("error"))
        .expect("Should find 'error' segment");

    println!("\n✅ Verification:");
    println!("  Strikethrough disabled to avoid 256-color conflicts");
    println!(
        "  error[E0425] segment has strikethrough={}",
        error_segment.style.strikethrough
    );

    assert!(
        !error_segment.style.strikethrough,
        "error[E0425] should NOT have strikethrough"
    );

    // Should have bold attribute
    assert!(error_segment.style.bold, "error[E0425] should be bold");
}
