[2025-12-10 13:08] - Updated by Junie
{
    "TYPE": "new instructions",
    "CATEGORY": "planning docs",
    "ERROR": "-",
    "NEW INSTRUCTION": "WHEN request mentions Junie agent implementation plan THEN Read docs folder design and implementation plans and produce a detailed step-by-step implementation plan"
}

[2025-12-10 13:15] - Updated by Junie
{
    "TYPE": "new instructions",
    "CATEGORY": "implementation workflow",
    "ERROR": "-",
    "NEW INSTRUCTION": "WHEN request references docs/juni-implementation-plan.md THEN Implement next task, keep tests green, update task status"
}

[2025-12-10 13:50] - Updated by Junie
{
    "TYPE": "new instructions",
    "CATEGORY": "naming conventions",
    "ERROR": "-",
    "NEW INSTRUCTION": "WHEN crate named \"core\" exists THEN rename crate to \"domain\" and update all references"
}

[2025-12-10 17:15] - Updated by Junie
{
    "TYPE": "correction",
    "CATEGORY": "planning docs",
    "ERROR": "Phase 1 was skipped and progress not reflected in plan",
    "NEW INSTRUCTION": "WHEN executing any phase from juni-implementation-plan.md THEN update phase status in docs/juni-implementation-plan.md immediately"
}

[2025-12-10 17:20] - Updated by Junie
{
    "TYPE": "new instructions",
    "CATEGORY": "implementation workflow",
    "ERROR": "-",
    "NEW INSTRUCTION": "WHEN request contains \"continue with phase\" THEN implement that phase and update plan status"
}

[2025-12-11 06:28] - Updated by Junie
{
    "TYPE": "correction",
    "CATEGORY": "implementation workflow",
    "ERROR": "Trigger phrase mismatch for continue request",
    "NEW INSTRUCTION": "WHEN request contains \"continue to phase\" THEN implement that phase and update plan status"
}

[2025-12-11 06:39] - Updated by Junie
{
    "TYPE": "new instructions",
    "CATEGORY": "implementation workflow",
    "ERROR": "-",
    "NEW INSTRUCTION": "WHEN request contains \"continue on with phase\" THEN implement that phase and update plan status"
}

[2025-12-11 09:56] - Updated by Junie
{
    "TYPE": "preference",
    "CATEGORY": "code documentation",
    "EXPECTATION": "Top-of-file documentation in main.rs and lib.rs explaining the crate and its purpose.",
    "NEW INSTRUCTION": "WHEN creating or updating main.rs or lib.rs THEN add top doc comment explaining crate purpose and role"
}

[2025-12-11 09:59] - Updated by Junie
{
    "TYPE": "preference",
    "CATEGORY": "code documentation",
    "EXPECTATION": "Top-of-file documentation in main.rs and lib.rs that states the crate and explains its purpose.",
    "NEW INSTRUCTION": "WHEN creating or updating any main.rs or lib.rs THEN add top-of-file doc explaining crate and purpose"
}

[2025-12-11 12:04] - Updated by Junie
{
    "TYPE": "preference",
    "CATEGORY": "auth security defaults",
    "EXPECTATION": "Operating without signature validation must be clearly indicated with a WARNING log, and the default runtime should use signature validation. Disabling validation for local runs must require an explicit flag or environment variable.",
    "NEW INSTRUCTION": "WHEN signature verification is disabled at runtime THEN log a WARNING stating non-production, no-signature mode"
}

[2025-12-11 12:31] - Updated by Junie
{
    "TYPE": "preference",
    "CATEGORY": "auth security defaults",
    "EXPECTATION": "Running without signature validation must log a WARNING, and default runtime must enable signature validation; disabling requires an explicit flag or environment variable.",
    "NEW INSTRUCTION": "WHEN no explicit disable flag is set THEN enable signature verification by default"
}

