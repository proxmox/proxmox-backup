Ext.define('LtoTapeType', {
    extend: 'Ext.form.field.ComboBox',
    alias: 'widget.ltoTapeType',

    editable: false,

    displayField: 'text',
    valueField: 'value',
    queryMode: 'local',

    store: {
	field: ['value', 'text'],
	data: [
	    { value: 'L9', text: "LTO-9" },
	    { value: 'LZ', text: "LTO-9 (WORM)" },
	    { value: 'L8', text: "LTO-8" },
	    { value: 'LY', text: "LTO-8 (WORM)" },
	    { value: 'L7', text: "LTO-7" },
	    { value: 'LX', text: "LTO-7 (WORM)" },
	    { value: 'L6', text: "LTO-6" },
	    { value: 'LW', text: "LTO-6 (WORM)" },
	    { value: 'L5', text: "LTO-5" },
	    { value: 'LV', text: "LTO-5 (WORM)" },
	    { value: 'L4', text: "LTO-4" },
	    { value: 'LU', text: "LTO-4 (WORM)" },
	    { value: 'L3', text: "LTO-3" },
	    { value: 'LT', text: "LTO-3 (WORM)" },
	    { value: 'CU', text: "Cleaning Unit" },
	],
    },
});
