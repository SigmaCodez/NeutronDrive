# Import Google Photos from Takeout

NeutronDrive beta 0.1 can upload photos and videos from an **extracted Google
Takeout export** into Proton Photos. It keeps original media bytes unchanged and
uses matching Google JSON sidecars for capture time and location when available.

## Prepare the export

1. Export Google Photos using Google Takeout.
2. Download **every archive part**. Extract ZIP/TGZ files yourself, combining their
   folders into one export folder. Keep the JSON sidecars beside the media files.
3. Keep the original export until you have checked the imported photos in Proton.
   NeutronDrive does not delete source files.

NeutronDrive does not extract archives. Symbolic links and special files are
ignored. The scanner accepts common photo/video extensions; actual decoding,
preview and upload support also depend on the Proton SDK.

## Review and import

1. Sign in to the intended Proton account.
2. Open **Photos → Google Takeout import** and choose the extracted export folder.
3. Review the media/video count, duplicate count, ignored files and metadata
   warnings. Nothing is uploaded before you confirm.
4. Confirm to upload into **Proton Photos**. Keep NeutronDrive running; closing to
   the tray is fine. Refresh Photos afterward to see new uploads.

The scanner reads the source files to calculate hashes, so scanning a large
export can take time. Identical content within the export is grouped. Existing
Proton Photos are checked by name/content through the SDK before upload; this is
not a global duplicate finder for differently named copies.

## Metadata and interruptions

Exact JSON sidecar names are preferred, including `.supplemental-metadata.json`.
A unique title match can associate truncated sidecar filenames. Missing or
ambiguous JSON metadata falls back to the SDK's embedded metadata handling and
is reported as a warning. Valid capture dates and GPS coordinates are imported;
albums, comments, descriptions and edited/original relationships are not rebuilt.

You can cancel the import. If a file fails, the batch stops and reports uploaded,
skipped, failed and remaining counts. A changed source file or account also stops
the operation. You can select the same export again; matching completed uploads
are checked before another upload. There is no persistent import queue or automatic
resume after quitting/restarting the app.

For large exports, work in batches: the scanner limits a selection to 100,000
unique media files, 500,000 directory entries and 64 directory levels. JSON limits
are 1 MiB per file and 64 MiB per directory.

The release includes synthetic importer and SDK contract tests. A real-account
Takeout upload has not been validated for this release; verify a small batch first.
