# i18n

Project-local UI translation overrides.

Default UI language is `en-US`. You can change it via `/settings lang <auto|en|zh-CN>` or by setting
`STAR_UI_LANGUAGE`.

Files use JSON with string keys:
```
{
  "ui.status.start": "Status: processing",
  "cmd.settings.lang.set": "UI language set to {lang} (effective: {resolved})"
}
```

Load order (lowest to highest priority):
1. `./i18n/<lang>.json`
2. `~/.star/i18n/<lang>.json`
3. `./.star/i18n/<lang>.json`
