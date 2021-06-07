Ext.define('PBS.form.DriveSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsDriveSelector',

    allowBlank: false,
    autoSelect: false,

    displayField: 'name',
    valueField: 'name',
    value: null,

    store: {
	proxy: {
	    type: 'proxmox',
	    url: '/api2/json/tape/drive',
	},
	autoLoad: true,
	sorters: 'name',
    },

    listConfig: {
	width: 450,
	columns: [
	    {
		text: gettext('Name'),
		dataIndex: 'name',
		sortable: true,
		flex: 3,
		renderer: Ext.String.htmlEncode,
	    },
	    {
		text: gettext('Vendor'),
		dataIndex: 'vendor',
		sortable: true,
		flex: 2,
		renderer: Ext.String.htmlEncode,
	    },
	    {
		text: gettext('Model'),
		dataIndex: 'model',
		sortable: true,
		flex: 3,
		renderer: Ext.String.htmlEncode,
	    },
	    {
		text: gettext('Serial'),
		dataIndex: 'serial',
		sortable: true,
		flex: 3,
		renderer: Ext.String.htmlEncode,
	    },
	],
    },

    initComponent: function() {
	let me = this;

	if (me.changer) {
	    me.store.proxy.extraParams = {
		changer: me.changer,
	    };
	} else {
	    me.store.proxy.extraParams = {};
	}

	me.callParent();
    },
});

