Ext.define('PageCalibration', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pageCalibration',

    layout: {
	type: 'hbox',
	align: 'stretch',
	pack: 'start',
    },

    getValues: function() {
	let me = this;

	let values = {};

	Ext.Array.each(me.query('[isFormField]'), function(field) {
	    if (field.isValid()) {
		let data = field.getSubmitData();
		Ext.Object.each(data, function(name, val) {
		    let parsed = parseFloat(val, 10);
		    values[name] = isNaN(parsed) ? val : parsed;
		});
	    }
	});

	if (values.d_x === undefined) { return; }
	if (values.d_y === undefined) { return; }
	if (values.s_x === undefined) { return; }
	if (values.s_y === undefined) { return; }

	scalex = 100/values.d_x;
	scaley = 100/values.d_y;

	let offsetx = ((50 - values.s_x) - (50*scalex - 50))/scalex;
	let offsety = ((50 - values.s_y) - (50*scaley - 50))/scaley;

	return {
	    scalex: scalex,
	    scaley: scaley,
	    offsetx: offsetx,
	    offsety: offsety,
	};
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	control: {
	    'field': {
		change: function() {
		    let view = this.getView();
		    let param = view.getValues();
		    view.fireEvent("calibrationchanged", param);
		},
	    },
	},
    },

    items: [
	{
	    border: false,
	    layout: {
		type: 'vbox',
		align: 'stretch',
		pack: 'start',
	    },
	    items: [
		{
		    xtype: 'displayfield',
		    value: 'a4',
		    fieldLabel: 'Start Offset Sx (mm)',
		    labelWidth: 150,
		    value: 50,
		},
		{
		    xtype: 'displayfield',
		    value: 'a4',
		    fieldLabel: 'Length Dx (mm)',
		    labelWidth: 150,
		    value: 100,
		},
		{
		    xtype: 'displayfield',
		    value: 'a4',
		    fieldLabel: 'Start Offset Sy (mm)',
		    labelWidth: 150,
		    value: 50,
		},
		{
		    xtype: 'displayfield',
		    value: 'a4',
		    fieldLabel: 'Length Dy (mm)',
		    labelWidth: 150,
		    value: 100,
		},
	    ],
	},
	{
	    border: false,
	    margin: '0 0 0 20',
	    layout: {
		type: 'vbox',
		align: 'stretch',
		pack: 'start',
	    },
	    items: [
		{
		    xtype: 'numberfield',
		    value: 'a4',
		    name: 's_x',
		    fieldLabel: 'Meassured Start Offset Sx (mm)',
		    allowBlank: false,
		    labelWidth: 200,
		},
		{
		    xtype: 'numberfield',
		    value: 'a4',
		    name: 'd_x',
		    fieldLabel: 'Meassured Length Dx (mm)',
		    allowBlank: false,
		    labelWidth: 200,
		},
		{
		    xtype: 'numberfield',
		    value: 'a4',
		    name: 's_y',
		    fieldLabel: 'Meassured Start Offset Sy (mm)',
		    allowBlank: false,
		    labelWidth: 200,
		},
		{
		    xtype: 'numberfield',
		    value: 'a4',
		    name: 'd_y',
		    fieldLabel: 'Meassured Length Dy (mm)',
		    allowBlank: false,
		    labelWidth: 200,
		},
	    ],
	},
    ],
});
