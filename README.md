Photoshop Scripts: `C:\Program Files\Adobe\Adobe Photoshop 2020\Presets\Scripts`

Illustrator Scripts: `C:\Program Files\Adobe\Adobe Illustrator 2020\Presets\en_US\Scripts`

After Effects Scripts: `C:\Program Files\Adobe\Adobe After Effects 2020\Support Files\Scripts`

Add author tag to rendered animated sticker file:

`C:\Program Files (x86)\Common Files\Adobe\CEP\extensions`

`jsx/renderManager.jsx` > Add _author: "@<your username>", to exportData

`static/js/main.something.js` >
    Edit "{ level: zlib.Z_BEST_COMPRESSION }" to "{ level: zlib.Z_BEST_COMPRESSION, memLevel: 9, strategy: zlib.Z_FILTERED }"
    Edit "return !showOnlySelected || showOnlySelected && item.selected;" to:
		if(item.name.indexOf(' Fill') + item.name.indexOf(' Border') + item.name.indexOf(' Main') + item.name.indexOf('Pre-comp') + item.name.indexOf(' Comp') > -1)
			return false;
	    return !showOnlySelected || showOnlySelected && item.selected;

regedit > HKEY_CURRENT_USER/Software/Adobe/CSXS.8, then add a new entry PlayerDebugMode of type "string" with the value of "1".