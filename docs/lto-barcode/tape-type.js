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
	    { value: 'L8', text: "LTO-8" },
	    { value: 'L7', text: "LTO-7" },
	    { value: 'L6', text: "LTO-6" },
	    { value: 'L5', text: "LTO-5" },
	    { value: 'L4', text: "LTO-4" },
	    { value: 'L3', text: "LTO-3" },
	    { value: 'CU', text: "Cleaning Unit" },
	],
    },
});
