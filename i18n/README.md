# i18n

UI translation dictionaries. **This directory is the single source of truth for every
user-visible string** — call sites only carry the key:

```rust
i18n::t1("ui.status.start")   // → "Status: processing" / "状态：开始处理"
```

To change a string, edit this file. Editing a call site does nothing: the lookup is
dictionary-first, and the dictionary is embedded in the binary.

Files use JSON with string keys:
```
{
  "ui.status.start": "Status: processing",
  "cmd.settings.lang.set": "UI language set to {lang} (effective: {resolved})"
}
```

**Both dictionaries must carry the same key set.** A key present in one but not the
other shows up as the raw key string in that language. `core::i18n`'s test suite
enforces this, along with "every key used in `src/` exists here" and "newlines are
real newlines, not the two-character sequence `\n`".

Values with interpolation use `{0}` / `{1}` for positional placeholders, or named
ones like `{name}` / `{count}` when there is only one.

Load order (lowest to highest priority):
1. Embedded in the binary (`rust_embed`, same mechanism as the system prompts)
2. `./i18n/<lang>.json`
3. `~/.star/i18n/<lang>.json`
4. `./.star/i18n/<lang>.json`

The embedded copy is what ships — the release tarball contains the binary, LICENSE
and README, but not this directory. External dictionaries let users override
translations without rebuilding.
