Ext.define('PBS.form.ChangerSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsChangerSelector',

    allowBlank: false,
    displayField: 'name',
    valueField: 'name',
    value: null,
    multiSelect: false,


    store: {
	proxy: {
	    type: 'proxmox',
	    url: '/api2/json/tape/changer',
	},
	autoLoad: true,
	sorter: 'name',
    },

    listConfig: {
	columns: [
	    {
		text: gettext('Name'),
		dataIndex: 'name',
		sortable: true,
		flex: 1,
		renderer: Ext.String.htmlEncode,
	    },
	    {
		text: gettext('Path'),
		sortable: true,
		dataIndex: 'path',
		hidden: true,
		flex: 1,
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
});
