Ext.define('LabelSetupPanel', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.labelSetupPanel',

    layout: {
	type: 'hbox',
	align: 'stretch',
	pack: 'start',
    },

    getValues: function() {
	let me = this;

	let values = {};

	Ext.Array.each(me.query('[isFormField]'), function(field) {
	    let data = field.getSubmitData();
	    Ext.Object.each(data, function(name, val) {
		let parsed = parseInt(val, 10);
		values[name] = isNaN(parsed) ? val : parsed;
	    });
	});

	return values;
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function() {
	    let me = this;
	    let view = me.getView();
	    let list = view.down("labelList");
	    let store = list.getStore();
	    store.on('datachanged', function(store) {
		view.fireEvent("listchanged", store);
	    });
	    store.on('update', function(store) {
		view.fireEvent("listchanged", store);
	    });
	},

	onAdd: function() {
	    let list = this.lookupReference('label_list');
	    let view = this.getView();
	    let params = view.getValues();
	    list.getStore().add(params);
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
		    xtype: 'prefixfield',
		    name: 'prefix',
		    value: 'TEST',
		    fieldLabel: 'Prefix',
		},
		{
		    xtype: 'ltoTapeType',
		    name: 'tape_type',
		    fieldLabel: 'Type',
		    value: 'L8',
		},
		{
		    xtype: 'ltoLabelStyle',
		    name: 'mode',
		    fieldLabel: 'Mode',
		    value: 'color',
		},
		{
		    xtype: 'numberfield',
		    name: 'start',
		    fieldLabel: 'Start',
		    minValue: 0,
		    allowBlank: false,
		    value: 0,
		},
		{
		    xtype: 'numberfield',
		    name: 'end',
		    fieldLabel: 'End',
		    minValue: 0,
		    emptyText: 'Fill',
		},
		{
		    xtype: 'button',
		    text: 'Add',
		    handler: 'onAdd',
		},
	    ],
	},
	{
	    margin: "0 0 0 10",
	    xtype: 'labelList',
	    reference: 'label_list',
	    flex: 1,
	},
    ],
});
