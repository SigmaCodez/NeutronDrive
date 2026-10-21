# System language and offline catalogs

The desktop selects its language automatically on startup. No network service,
model, translation library or extra account is needed at runtime. Unsupported
languages and missing/invalid individual translations fall back to English.
Restart the application after changing the desktop's language.

The language set matches Proton Drive for Windows `SupportedLanguages` in
[LanguageService.cs](https://github.com/ProtonDriveApps/windows-drive/blob/ba24801b8e566b6d0967b602d90832389152efe7/src/Proton.Drive.App/Localization/LanguageService.cs),
checked on 2026-10-21. Only the language selection was referenced; Proton's
application code and resource catalogs were not copied.

Supported catalogs: English (`en`), German (`de`), French (`fr`), Dutch (`nl`),
Spanish (`es`), Latin American Spanish (`es-419`), Italian (`it`), Polish (`pl`),
Brazilian Portuguese (`pt-BR`), Russian (`ru`), Turkish (`tr`), Catalan (`ca-ES`),
Czech (`cs-CZ`), Danish (`da-DK`), Hungarian (`hu`), Indonesian (`id`), Norwegian
Bokmål (`nb-NO`), European Portuguese (`pt`), Romanian (`ro`), Slovak (`sk`),
Swedish (`sv-SE`), Greek (`el-GR`), Belarusian (`be-BY`), Simplified Chinese
(`zh-CN`) and Traditional Chinese (`zh-TW`).

On Linux, the effective message locale follows `LC_ALL`, `LC_MESSAGES`, then
`LANG`; `LANGUAGE` supplies an ordered UI preference list, except in the C/POSIX
locale, which uses English. With no locale environment, Qt's system UI languages
are used. Encodings and POSIX modifiers are stripped. Regional German/French/
English variants map to their base language; Latin American Spanish, Brazilian
Portuguese, and Chinese script/region variants retain their distinct catalogs.

The presentation layer translates labels, prompts, activity/status text and
known diagnostic templates. Paths, file names, URLs, command identifiers and
persisted/IPC state remain unchanged. Folder icons use a typed boolean rather
than comparing translated text. Qt's bundled base/declarative translations cover
standard dialogs where Qt provides them. Unknown external diagnostic messages
are retained verbatim. Numeric file sizes and dates use the selected Qt locale.

`src/neutrondrive/locales/en.json` is the required fallback catalog. All 25 JSON
catalogs ship as package data in wheels and in the existing native, Flatpak and
Snap staging pipeline. The UI contains no translation download or telemetry.

## Translation provenance and review

English UI text and German-to-English diagnostic/help text were edited directly.
Short navigation/action terminology was separately edited in all supported base
languages. Other initial translations were generated locally with the
MIT-licensed M2M100 model; model weights and inference dependencies are build-time
scratch data and are not distributed with NeutronDrive. Final model reference:
`JustFrederik/m2m_100_1.2b_ct2_int8`, revision
`0e07f3608b9be1a81df0813b53c3e338ecc3890a`, derived from Meta M2M100.

The user permitted Google Translate for fixed public UI text, but requests were
rate-limited and no Google translations were used. Local generation processed
only fixed source strings, never user files, account data or live diagnostics.

These are initial translations, not professionally certified translations.
A complete native-speaker review of all longer explanations and diagnostics is
still needed. Technical completeness and placeholder checks do not establish
linguistic quality. Corrections should preserve each message's meaning, especially
negations and data-preservation warnings.

## Maintaining catalogs

Add new UI messages through `tr()` in Python or `window.tr()` in QML. Keep action
IDs and user-controlled values outside translations. Use named placeholders
`{p0}`, `{p1}`, etc. and interpolate only after translation. Every locale must
contain exactly the English catalog's keys and preserve its placeholder set,
format specifications and conversions. Unsupported or malformed optional entries
must never crash a dialog or cause a fallback to German.

Run `python3 -m unittest discover -s tests -p 'test_i18n*.py' -v` for locale and
catalog checks. Enable `NEUTRONDRIVE_UI_TESTS=1` with the bundled Qt runtime for the
26-locale window test (25 supported plus unsupported Japanese). Existing German
UI assertions pin their locale rather than relying on the build runner's locale.

## Validation on 2026-10-21

All 340 local tests passed with no skips, including real FUSE and isolated D-Bus,
Qt UI tests, locale resolution, 25 complete catalogs, named-placeholder integrity,
and native/Snap staging. The UI test opened all six pages in every supported
language plus an unsupported Japanese locale (English fallback), exercised a
rename dialog and checked that user file names and action IDs stay unchanged.

After the final text-only arrow corrections, all eight localization tests passed
again, including the 26 window configurations. Each catalog contains 579 messages.
A built wheel contains all 25 JSON catalogs with byte-for-byte source matches.
Native/Snap staging also verified both Qt base and declarative translations.
French and Traditional Chinese screenshots were visually inspected. These checks
establish technical integration, not a complete native-speaker language review.

This localization is included in alpha6. See the [release validation record](VALIDATION.md)
for package checks and the distinction between technical coverage and linguistic review.
