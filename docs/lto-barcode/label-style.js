Ext.define('LtoLabelStyle', {
    extend: 'Ext.form.field.ComboBox',
    alias: 'widget.ltoLabelStyle',

    editable: false,

    displayField: 'text',
    valueField: 'value',
    queryMode: 'local',

    store: {
	field: ['value', 'text'],
	data: [
	    { value: 'simple', text: "Simple" },
	    { value: 'color', text: 'Color (frames with color)' },
	    { value: 'frame', text: 'Frame (no color)' },
	    { value: 'placeholder', text: 'Placeholder (empty)' },
	],
    },
});
