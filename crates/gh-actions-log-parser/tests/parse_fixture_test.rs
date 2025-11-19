use gh_actions_log_parser::{job_log_to_tree, parse_workflow_logs};
use std::io::Write;

#[test]
fn test_parse_fixture_file() {
    // Read the fixture file
    let content = std::fs::read("tests/fixtures/job-logs.txt").unwrap();

    // Create a minimal ZIP with the fixture content
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::FileOptions::<()>::default();
    zip.start_file("test.txt", options).unwrap();
    zip.write_all(&content).unwrap();
    let zip_data = zip.finish().unwrap().into_inner();

    // Parse it
    let parsed = parse_workflow_logs(&zip_data).unwrap();
    assert_eq!(parsed.jobs.len(), 1);

    let job_log = &parsed.jobs[0];
    println!("\n=== Job: {} ===", job_log.name);
    println!("Total lines: {}", job_log.lines.len());

    // Convert to tree
    let job_node = job_log_to_tree(job_log.clone());
    println!("\nSteps found: {}", job_node.steps.len());

    for (i, step) in job_node.steps.iter().enumerate() {
        println!(
            "\n  Step {}: '{}' ({} lines, {} errors)",
            i,
            step.name,
            step.lines.len(),
            step.error_count
        );

        // Show first 5 and last 5 lines of each step
        for (j, line) in step.lines.iter().take(5).enumerate() {
            println!(
                "    Line {}: '{}'",
                j,
                line.display_content.chars().take(80).collect::<String>()
            );
        }
        if step.lines.len() > 10 {
            println!("    ... ({} more lines) ...", step.lines.len() - 10);
            for (j, line) in step.lines.iter().skip(step.lines.len() - 5).enumerate() {
                println!(
                    "    Line {}: '{}'",
                    step.lines.len() - 5 + j,
                    line.display_content.chars().take(80).collect::<String>()
                );
            }
        } else if step.lines.len() > 5 {
            for (j, line) in step.lines.iter().skip(5).enumerate() {
                println!(
                    "    Line {}: '{}'",
                    5 + j,
                    line.display_content.chars().take(80).collect::<String>()
                );
            }
        }
    }

    // Specifically check the "Run cargo check" step
    let cargo_check_step = job_node
        .steps
        .iter()
        .find(|s| s.name.contains("cargo check"))
        .expect("Should find 'Run cargo check' step");

    println!("\n=== Cargo Check Step Details ===");
    println!("Name: '{}'", cargo_check_step.name);
    println!("Lines: {}", cargo_check_step.lines.len());

    // Verify it contains the actual cargo output (not just metadata)
    let has_updating = cargo_check_step
        .lines
        .iter()
        .any(|line| line.display_content.contains("Updating"));
    let has_downloading = cargo_check_step
        .lines
        .iter()
        .any(|line| line.display_content.contains("Downloading"));
    let has_error = cargo_check_step
        .lines
        .iter()
        .any(|line| line.display_content.contains("error"));

    assert!(
        has_updating,
        "Should contain 'Updating crates.io index' output"
    );
    assert!(has_downloading, "Should contain 'Downloading' output");
    assert!(has_error, "Should contain compilation errors");

    // Verify the step name doesn't include ##[group] prefix
    assert_eq!(
        cargo_check_step.name, "Run cargo check",
        "Step name should be 'Run cargo check' without ##[group] prefix"
    );

    // Verify ##[group] lines are marked as metadata and hidden
    let has_group_line = cargo_check_step
        .lines
        .iter()
        .any(|line| line.display_content.contains("##[group]"));
    assert!(
        !has_group_line,
        "##[group] lines should be marked as metadata and not appear in step lines"
    );

    // Verify [command] prefix is removed from command lines
    let git_version_step = job_node
        .steps
        .iter()
        .find(|s| s.name.contains("Git version"))
        .expect("Should find 'Getting Git version info' step");

    let has_command_prefix = git_version_step
        .lines
        .iter()
        .any(|line| line.display_content.starts_with("[command]"));
    assert!(
        !has_command_prefix,
        "[command] prefix should be removed from display_content"
    );

    // Verify command lines are marked as is_command
    let has_command_lines = git_version_step.lines.iter().any(|line| line.is_command);
    assert!(
        has_command_lines,
        "Lines with [command] prefix should be marked as is_command=true"
    );

    println!("\n✅ All fixes verified:");
    println!("  - ##[group] lines are hidden (marked as metadata)");
    println!("  - [command] prefixes are removed from display");
    println!("  - Command lines are marked for special styling");
}
