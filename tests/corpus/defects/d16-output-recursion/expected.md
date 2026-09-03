D16 is a directory-structural defect, not a text transform, so this file
states the correct behavior in prose rather than as a renderable fixture.

A correct implementation must do ONE of the following when the resolved
output directory is a subdirectory of the input directory (or discovers,
mid-walk, that it has become one):

(a) **Refuse and error out.** Detect at the start of `convert_directory`
    (convert.py:222) that `effective_output_dir` is nested inside `input_dir`
    (e.g. via `os.path.commonpath` / `os.path.realpath` containment check)
    and raise a clear error before creating the directory or converting any
    file, rather than silently proceeding. This is the simpler, safer fix and
    matches the threat model recorded in phase-01 (input repositories are
    untrusted; a malicious or careless `-o` pointing inside the input tree
    should not silently corrupt the run).

(b) **Exclude the output directory from traversal.** Make `discover_files`
    (convert.py:48-99) and `_copy_assets` (convert.py:335-368) share one
    skip-set that always includes the resolved `effective_output_dir` (not
    just the fixed `skip_dirs` name list), so that even when the output
    directory is nested inside the input directory, neither the markdown
    discovery pass nor the asset-copy pass ever descends into it. This
    preserves the ability to (deliberately or not) nest output inside input
    without duplicating on every run.

Either outcome is acceptable; Phase 3 (path safety) is where the plan decides
between them. This fixture exists only to prove the current defect — that
`discover_files` and `_copy_assets` keep separate skip-sets and neither
excludes the output directory — reproducibly, per `python-actual.txt`.
