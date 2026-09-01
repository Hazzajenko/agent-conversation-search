# Centralize Current Session resolution

One current-context module resolves the top-level Session, calling thread, and Current Session Family for every command. Claude and Codex supply Harness-specific adapters behind this interface; commands do not inspect environment variables or reconstruct ancestry themselves. The shared seam keeps capability detection, ambiguity handling, Store lookup, and test fixtures local while supporting `current`, `show`, `search`, failure analysis, and `export` consistently.
