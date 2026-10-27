# Keyboard navigation

## Files and folders

| Key | Action |
| --- | --- |
| Ctrl+F | Focus folder search and select the existing query |
| Escape in search | Clear the query; press again to return focus to the file view |
| Down in search | Move focus to the file view without clearing the query |
| Home / End in the file view | Select the first / last visible entry |
| Arrow keys in the file view | Move between entries |
| Shift+arrow / Shift+Home / Shift+End | Extend or shrink a continuous selection from its anchor |
| Shift+click | Select the range between the anchor and the clicked entry |
| Ctrl+click | Toggle an individual entry |
| Escape in the file view | Clear the selection while keeping keyboard focus |
| Space in the file view | Toggle the focused entry's selection |
| Ctrl+A in the file view | Select all visible entries |
| Enter in the file view | Activate the focused entry |

Search applies to the current folder. It uses Unicode case folding, so `Straße`
and `STRASSE` have the same results. Editing a query into an equivalent spelling
keeps the selection and existing file delegates. A different filter clears the
selection so hidden items cannot be included in a subsequent action.

Find is available on the file page when the app is idle and no dialog is open.
Typing in a sharing, setup or confirmation dialog does not activate folder search.
The list and grid use the same navigation behavior. Range selection keeps a fixed
anchor: reversing direction shrinks the range without leaving gaps. A new single
selection or Ctrl+click sets a new anchor. Changing the folder or filter resets it. Empty search avoids unnecessary
case folding of every filename when rendering a folder.

For photo preview shortcuts, see [Photo viewer](PHOTO-VIEWER.md).

These changes are on the development branch and are not yet included in the
published 1.0.1 packages.
