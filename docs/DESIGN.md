# Interface design

NeutronDrive uses one light/dark palette in `Main.qml` and shared Qt Quick
components for buttons, text fields, checkboxes, selectors, menus and scrollbars.
Reuse these components when adding screens instead of introducing a second style.

- Buttons use a 40 px minimum height; inputs and selectors use 44 px.
- Interactive surfaces use 10 px corners, with 12–18 px on menus and larger cards.
- Purple identifies the primary action, selection and keyboard focus.
- Disabled, hover, pressed and keyboard-focus states are explicit in both themes.
- All user-controlled text is rendered as plain text; labels keep their existing
  translations and accessible names.
- Application dialogs use the same surfaces and controls. Operating-system file
  pickers and desktop tray menus retain the system's native presentation.
- Photo tiles decode at their display size and screen density. Model changes
  received during a flick are applied when movement finishes. Selection and
  scroll position are preserved; requests inspect the entire visible range for
  missing previews. Preview retention remains bounded.

The UI regression run checks setup, themes, settings, account controls, keyboard
navigation, file grids, photo selection/viewer, large photo-model updates and
updates received during a flick. Synthetic tests do not measure real cloud
latency or guarantee a frame rate on every graphics driver.
