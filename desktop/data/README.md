# Unicode search data

The native Rust file browser uses **default full case folding** (C + F mappings),
not Qt's simple case folding. This retains searches such as `STRASSE` → `Straße`.
Unicode 16.0.0 matches the former runtime's Unicode database. Runtime parsing is
Rust-only and cached; builds and execution require neither downloads nor Python.

Source: https://www.unicode.org/Public/16.0.0/ucd/CaseFolding.txt

SHA-256: `6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb`

License: `UNICODE-LICENSE.txt`, obtained from https://www.unicode.org/license.txt
(SHA-256 `e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96`).

The migration validation compared all 1,112,064 Unicode scalar values against
Python's Unicode 16 `str.casefold()`, with no differences. Native tests also cover
multi-character folds, final sigma, dotted I, Cherokee and unmapped characters.
No normalization or locale-specific Turkic folding is introduced.
