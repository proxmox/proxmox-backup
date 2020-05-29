Ext.define('PBS.form.RemoteSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsRemoteSelector',

    allowBlank: false,
    autoSelect: false,
    valueField: 'name',
    displayField: 'name',

    store: {
	model: 'pmx-remotes',
	autoLoad: true,
	sorters: 'name',
    },

    listConfig: {
	columns: [
	    {
		header: gettext('Remote'),
		sortable: true,
		dataIndex: 'name',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Host'),
		sortable: true,
		dataIndex: 'host',
		flex: 1,
	    },
	    {
		header: gettext('User name'),
		sortable: true,
		dataIndex: 'userid',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },
});
