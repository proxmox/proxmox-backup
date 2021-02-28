Ext.define('PageLayoutPanel', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pageLayoutPanel',

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
		    values[name] = val;
		});
	    }
	});

	let paper_size = values.paper_size || 'a4';

	let param = Ext.apply({}, paper_sizes[paper_size]);
	if (param === undefined) {
	    throw `unknown paper size ${paper_size}`;
	}

	param.paper_size = paper_size;

	Ext.Object.each(values, function(name, val) {
	    let parsed = parseFloat(val, 10);
	    param[name] = isNaN(parsed) ? val : parsed;
	});

	return param;
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	control: {
	    'paperSize': {
		change: function(field, paper_size) {
		    let view = this.getView();
		    let defaults = paper_sizes[paper_size];

		    let names = [
			'label_width',
			'label_height',
			'margin_left',
			'margin_top',
			'column_spacing',
			'row_spacing',
		    ];
		    for (i = 0; i < names.length; i++) {
			let name = names[i];
			let f = view.down(`field[name=${name}]`);
			let v = defaults[name];
			if (v != undefined) {
			    f.setValue(v);
			    f.setDisabled(defaults.fixed);
			} else {
			    f.setDisabled(false);
			}
		    }
		},
	    },
	    'field': {
		change: function() {
		    let view = this.getView();
		    let param = view.getValues();
		    view.fireEvent("pagechanged", param);
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
		    xtype: 'paperSize',
		    name: 'paper_size',
		    value: 'a4',
		    fieldLabel: 'Paper Size',
		},
		{
		    xtype: 'numberfield',
		    name: 'label_width',
		    fieldLabel: 'Label width',
		    minValue: 70,
		    allowBlank: false,
		    value: 70,
		},
		{
		    xtype: 'numberfield',
		    name: 'label_height',
		    fieldLabel: 'Label height',
		    minValue: 15,
		    allowBlank: false,
		    value: 17,
		},
		{
		    xtype: 'checkbox',
		    name: 'label_borders',
		    fieldLabel: 'Label borders',
		    value: true,
		    inputValue: true,
		},
	    ],
	},
	{
	    border: false,
	    margin: '0 0 0 10',
	    layout: {
		type: 'vbox',
		align: 'stretch',
		pack: 'start',
	    },
	    items: [
		{
		    xtype: 'numberfield',
		    name: 'margin_left',
		    fieldLabel: 'Left margin',
		    minValue: 0,
		    allowBlank: false,
		    value: 0,
		},
		{
		    xtype: 'numberfield',
		    name: 'margin_top',
		    fieldLabel: 'Top margin',
		    minValue: 0,
		    allowBlank: false,
		    value: 4,
		},
		{
		    xtype: 'numberfield',
		    name: 'column_spacing',
		    fieldLabel: 'Column spacing',
		    minValue: 0,
		    allowBlank: false,
		    value: 0,
		},
		{
		    xtype: 'numberfield',
		    name: 'row_spacing',
		    fieldLabel: 'Row spacing',
		    minValue: 0,
		    allowBlank: false,
		    value: 0,
		},
	    ],
	},
    ],

});
