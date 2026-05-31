# Third-Party Notices

Document Finder is licensed under **GNU AGPL-3.0-or-later** (see [`LICENSE`](LICENSE)),
copyright Web World Wide. This file documents the licenses of the third-party
components it uses. It is informational and does not modify the terms of any
listed component or of Document Finder itself.

License inventory last audited: **2026-05-30**.

---

## AI model weights (downloaded at runtime, not bundled)

Document Finder does **not** ship or redistribute any model weights. The optional
local-LLM feature downloads weights on demand from Hugging Face to the user's
machine; the user obtains them directly from the model publisher under that
publisher's license. The catalog intentionally lists only permissively licensed
models compatible with this app's AGPL license:

| Model | Publisher | License | Source |
|---|---|---|---|
| Qwen 2.5 1.5B Instruct (GGUF, Q4_K_M) — *default* | Alibaba Cloud (Qwen) | Apache-2.0 | `Qwen/Qwen2.5-1.5B-Instruct-GGUF` |
| Qwen 2.5 0.5B Instruct (GGUF, Q4_K_M) | Alibaba Cloud (Qwen) | Apache-2.0 | `Qwen/Qwen2.5-0.5B-Instruct-GGUF` |

The text-embedding model (BGE-Small-EN-v1.5, MIT) is managed and fetched by the
`fastembed` library's own cache, also at runtime.

Each model's download is verified against a pinned SHA-256 hash
(`src-tauri/src/ai/registry.rs`) before use.

> Models with non-commercial or custom-restricted terms — e.g. Qwen's
> "qwen-research" (Qwen2.5-3B), Meta's Llama Community License, and Mistral's
> Research License (Ministral 3B/8B) — are deliberately **excluded** so the
> shipped defaults impose no extra restrictions beyond Document Finder's own.

---

## Rust dependencies (`src-tauri/`)

All transitive Rust dependencies are under permissive, AGPL-compatible licenses.
SPDX-expression breakdown (counts; `OR` expressions let the consumer pick the
permissive option):

```
MIT OR Apache-2.0 .......... 322      ISC ........................ 5
MIT ........................ 154      Unlicense OR MIT ........... 4
MIT/Apache-2.0 ............. 41       BSD-3-Clause ............... 4
Apache-2.0 OR MIT .......... 35       MPL-2.0 .................... 7
Unicode-3.0 ................ 18       Zlib ....................... 2
Apache-2.0 WITH LLVM-exc. .. 16       CC0-1.0 OR MIT-0 OR Apache . 2
Zlib OR Apache-2.0 OR MIT .. 11       BSL-1.0 (Boost) ............ 1 (as OR)
Apache-2.0 ................. 11       CDLA-Permissive-2.0 ........ 1
(plus assorted MIT/BSD/Apache/Zlib combinations)
```

No GPL-2.0-only, AGPL, SSPL, BUSL, CC-BY-NC, or proprietary licenses are present
(the single `AGPL-3.0-or-later` entry is Document Finder's own crate). MPL-2.0 and
the LLVM-exception are file-level licenses that do not extend to the combined
work. Notable bundled native libraries (all permissive): `llama-cpp-2` /
`llama.cpp` (MIT), ONNX Runtime via `ort` (MIT), `fastembed` (Apache-2.0),
`rusqlite` / bundled SQLite (public domain / MIT), `reqwest` + `rustls` (MIT /
Apache-2.0).

Reproduce the full per-crate list:

```bash
cargo metadata --format-version 1 --manifest-path src-tauri/Cargo.toml \
  | python -c "import sys,json;[print(p['name'],p['version'],p.get('license')) for p in json.load(sys.stdin)['packages']]"
```

---

## JavaScript / npm dependencies (`src/`, build tooling)

All JS dependencies are under permissive, AGPL-compatible licenses:

```
MIT ........ 258    BSD-3-Clause .... 7     MIT-0 ......... 1
ISC ........ 19     BlueOak-1.0.0 ... 5     Python-2.0 .... 1  (argparse)
Apache-2.0 . 18     MPL-2.0 ......... 2     CC-BY-4.0 ..... 1  (caniuse-lite)
BSD-2-Clause  8     Apache-2.0 OR MIT 3
```

Attribution notes: `caniuse-lite` (CC-BY-4.0) is build-time browser-support data;
`lightningcss` (MPL-2.0) is Tailwind's CSS engine (file-level copyleft, not
viral). No copyleft-incompatible licenses are present.

Reproduce: `pnpm licenses list`.

---

*Generated as part of a license-compatibility audit. To refresh after dependency
changes, re-run the commands above and update the breakdowns.*
