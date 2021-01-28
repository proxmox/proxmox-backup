Ext.define('PBS.form.TapeDevicePathSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsTapeDevicePathSelector',

    allowBlank: false,
    displayField: 'path',
    valueField: 'path',

    // type can be 'drives' or 'changers'
    type: 'drives',

    listConfig: {
	columns: [
	    {
		text: gettext('Path'),
		dataIndex: 'path',
		sortable: true,
		flex: 1,
		renderer: Ext.String.htmlEncode,
	    },
	    {
		text: gettext('Vendor'),
		dataIndex: 'vendor',
		sortable: true,
		flex: 1,
		renderer: Ext.String.htmlEncode,
	    },
	    {
		text: gettext('Model'),
		dataIndex: 'model',
		sortable: true,
		flex: 1,
		renderer: Ext.String.htmlEncode,
	    },
	    {
		text: gettext('Serial'),
		dataIndex: 'serial',
		sortable: true,
		flex: 1,
		renderer: Ext.String.htmlEncode,
	    },
	],
    },

    initComponent: function() {
	let me = this;
	if (me.type !== 'drives' && me.type !== 'changers') {
	    throw `invalid type '${me.type}'`;
	}

	let url = `/api2/json/tape/scan-${me.type}`;
	me.store = {
	    proxy: {
		type: 'proxmox',
		url,
	    },
	    autoLoad: true,
	};

	me.callParent();
    },
});
