[2025-12-10 12:55] - Updated by Junie - Error analysis
{
    "TYPE": "invalid args",
    "TOOL": "get_file_structure",
    "ERROR": "Cannot display file structure for this file",
    "ROOT CAUSE": "Called get_file_structure on a markdown file instead of a directory path.",
    "PROJECT NOTE": "Use get_file_structure or list_directory on the docs/ directory; use read_file for docs/implementation-plan.md or design docs.",
    "NEW INSTRUCTION": "WHEN target is a single file path THEN use read_file instead of get_file_structure"
}

[2025-12-11 08:29] - Updated by Junie - Error analysis
{
    "TYPE": "invalid args",
    "TOOL": "open_entire_file",
    "ERROR": "Path not found; auto-replaced with different file",
    "ROOT CAUSE": "Attempted to open non-existent adapter files under adapters/aws-dynamo; only the directory exists.",
    "PROJECT NOTE": "Adapter crates under adapters/* must contain a Cargo.toml; absence indicates the crate isn't created yet.",
    "NEW INSTRUCTION": "WHEN tool warns path not found and auto-replaced THEN list directory or search to validate path"
}

[2025-12-15 14:51] - Updated by Junie - Error analysis
{
    "TYPE": "invalid content",
    "TOOL": "apply_patch",
    "ERROR": "JSON code blocks contain comments; validator rejects them",
    "ROOT CAUSE": "Inserted JSON examples in Markdown with inline comments, violating strict JSON validation.",
    "PROJECT NOTE": "Docs are linted for strict JSON in code blocks; comments in JSON are disallowed.",
    "NEW INSTRUCTION": "WHEN adding JSON code blocks to docs THEN use strictly valid JSON without comments"
}

