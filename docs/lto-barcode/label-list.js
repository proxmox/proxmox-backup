Ext.define('LabelList', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.labelList',

    plugins: {
        ptype: 'cellediting',
        clicksToEdit: 1,
    },

    selModel: 'cellmodel',

    store: {
	field: [
	    'prefix',
	    'tape_type',
	    {
		type: 'integer',
		name: 'start',
	    },
	    {
		type: 'integer',
		name: 'end',
	    },
	],
	data: [],
    },

    listeners: {
	validateedit: function(editor, context) {
	    console.log(context.field);
	    console.log(context.value);
	    context.record.set(context.field, context.value);
	    context.record.commit();
	    return true;
	},
    },

    columns: [
	{
            text: 'Prefix',
            dataIndex: 'prefix',
	    flex: 1,
	    editor: {
		xtype: 'prefixfield',
		allowBlank: false,
	    },
	    renderer: function(value, metaData, record) {
		console.log(record);
		if (record.data.mode === 'placeholder') {
		    return "-";
		}
		return value;
	    },
	},
	{
            text: 'Type',
            dataIndex: 'tape_type',
	    flex: 1,
	    editor: {
		xtype: 'ltoTapeType',
		allowBlank: false,
	    },
	    renderer: function(value, metaData, record) {
		console.log(record);
		if (record.data.mode === 'placeholder') {
		    return "-";
		}
		return value;
	    },
	},
	{
            text: 'Mode',
            dataIndex: 'mode',
	    flex: 1,
	    editor: {
		xtype: 'ltoLabelStyle',
		allowBlank: false,
	    },
	},
	{
	    text: 'Start',
	    dataIndex: 'start',
	    flex: 1,
	    editor: {
		xtype: 'numberfield',
		allowBlank: false,
	    },
	},
	{
	    text: 'End',
	    dataIndex: 'end',
	    flex: 1,
	    editor: {
		xtype: 'numberfield',
	    },
	    renderer: function(value) {
		if (value === null || value === '' || value === undefined) {
		    return "Fill";
		}
		return value;
	    },
	},
	{
	    xtype: 'actioncolumn',
	    width: 75,
	    items: [
		{
		    tooltip: 'Move Up',
		    iconCls: 'fa fa-arrow-up',
		    handler: function(grid, rowIndex) {
			if (rowIndex < 1) { return; }
			let store = grid.getStore();
			let record = store.getAt(rowIndex);
			store.removeAt(rowIndex);
			store.insert(rowIndex - 1, record);
		    },
		},
		{
		    tooltip: 'Move Down',
		    iconCls: 'fa fa-arrow-down',
		    handler: function(grid, rowIndex) {
			let store = grid.getStore();
			if (rowIndex >= store.getCount()) { return; }
			let record = store.getAt(rowIndex);
			store.removeAt(rowIndex);
			store.insert(rowIndex + 1, record);
		    },
		},
		{
		    tooltip: 'Delete',
		    iconCls: 'fa fa-scissors',
		    //iconCls: 'fa critical fa-trash-o',
		    handler: function(grid, rowIndex) {
			grid.getStore().removeAt(rowIndex);
		    },
		},
	    ],
	},
    ],
});
