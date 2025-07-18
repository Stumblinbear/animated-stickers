### Note: There is no versioning due to scripting restrictions, so check back occasionally for updates.

Photoshop Scripts: `C:\Program Files\Adobe\Adobe Photoshop 2025\Presets\Scripts`

Illustrator Scripts: `C:\Program Files\Adobe\Adobe Illustrator 2025\Presets\en_US\Scripts`

After Effects Scripts: `C:\Program Files\Adobe\Adobe After Effects 2025\Support Files\Scripts`

## Optional Changes

The following changes are entirely optional. If you do apply any them, you must first allow After Effects to load modified scripts, otherwise it may not load them properly for _reasons_.

1. Open up the Windows Registry Editor by opening the Start Menu and typing "Registry Editor"
2. In the window that appears, use the left-hand sidebar to navigate to the following directory: `HKEY_CURRENT_USER/Software/Adobe/CSXS.12`
3. On the right, you should see a couple of entries called `(Default)` and `LogLevel`
4. Right click and select `New > String Value`
5. Rename it to `PlayerDebugMode`
6. Double click on the entry
7. In the window that appears, enter `1` as the "Value data" and hit OK

### Adding your username to the exported sticker

This will add a field to the exported sticker automatically when it's rendered. While this doesn't affect anything in practice, you'll know it's there—isn't that all that matters?

1. Edit `C:/Program Files/Common Files/Adobe/CEP/extensions/com.bodymovin.bodymovin/jsx/renderManager.jsx`
2. Find the `exportData` object
3. Add `_author: "@username",` to it

### Increase compression for smaller files

Due to the vectorization process, you'll end up with pretty large rendered sticker files. With a limit of 64kb (imposed by Telegram) you'll often bump up against this limit. You can give yourself a bit more leeway by adjusting the compression settings.

1. Edit `C:/Program Files/Common Files/Adobe/CEP/extensions/com.bodymovin.bodymovin/static/js/main.<something>.js`
2. Find the line that says `{ level: zlib.Z_BEST_COMPRESSION }`
3. Replace it with `{ level: zlib.Z_BEST_COMPRESSION, memLevel: 9, strategy: zlib.Z_FILTERED }`

### Hide compositions created by "Prepare for Render"

When running the "Prepare for Render" script with a "Full Mask" applied, it will create a few extra compositions in order to do the effect properly. Applying this edit will hide these extra compositions from the "Bodymovin for Telegram Stickers" window that you render your finished stickers from.

1. Edit `C:/Program Files/Common Files/Adobe/CEP/extensions/com.bodymovin.bodymovin/static/js/main.<something>.js`
2. Find the line that says `return !showOnlySelected || showOnlySelected && item.selected;`
3. Replace it with the following:

```js
if(item.name.indexOf(' Fill') + item.name.indexOf(' Border') + item.name.indexOf(' Main') + item.name.indexOf('Pre-comp') + item.name.indexOf(' Comp') > -1)
	return false;
return !showOnlySelected || showOnlySelected && item.selected;
```
