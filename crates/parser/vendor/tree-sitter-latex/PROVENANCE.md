## Upstream

Repository: `https://github.com/latex-lsp/tree-sitter-latex`

Commit: `fa8df448fc2c0192a8c2f8cfc97de53cb2b4ecb9`

Upstream package version: `0.6.0`

License: MIT

## Generation

Tree-sitter CLI: `0.24.1`

Command: `tree-sitter generate --abi 14 src/grammar.json`

Target parser ABI: 14

Intended Rust runtime: `tree-sitter 0.24.1` (compatible ABI range 13–14)

## Hashes

- `grammar.js`: `9f7dbb192076bebd6052355a4f6c2ce4b1d9d0184019113a1f019cf7b789082f`
- `src/grammar.json`: `617ee120cda4772e0d0998e21e7044f29c460a5f64e1c86c915e604a22e43616`
- `src/node-types.json`: `db931e1dddb6273d316db41664df4de1559ef2a803c52e97d217856f8db91351`
- `src/scanner.c`: `8a7475b893beb61bd263d3e940a9b78e5b14590d6cc0e0a503f61b92eab198cd`
- `src/parser.c`: `b3518a05a065210a37a3f2fcac7ee042a67c4b3edd6104e6d233c0ae608578ed`
- `src/tree_sitter/alloc.h`: `b29c1c9fb7cc82f58c84b376df1297d6e2737a1d655fd356db0859e3c29c2fea`
- `src/tree_sitter/array.h`: `4ff743903dc46f5db6aa54f31c6b4d160a8a9779e5b2ab1ee59ae7ebcd850ea1`
- `src/tree_sitter/parser.h`: `a1f6ef161fbaf48a0e10fca90ef5290a062462b307b3898aa562993853b9f80a`

## Reason for vendoring

The official Git revision omits generated `src/parser.c` while its Rust binding expects it. LaTeX Core reproducibly generates that file from the exact official structured grammar rather than consuming a semantically different third-party repackaging.

## Third-party package rejection

`codebook-tree-sitter-latex 0.6.1` is not used because its grammar differs from the selected upstream revision in underscore handling for labels.
