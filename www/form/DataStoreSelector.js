Ext.define('PBS.form.DataStoreSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsDataStoreSelector',

    allowBlank: false,
    autoSelect: false,
    valueField: 'store',
    displayField: 'store',

    store: {
	model: 'pbs-datastore-list',
	autoLoad: true,
	sorters: 'store',
    },

    listConfig: {
	columns: [
	    {
		header: gettext('Datastore'),
		sortable: true,
		dataIndex: 'store',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		sortable: true,
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Maintenance'),
		sortable: true,
		dataIndex: 'maintenance',
		renderer: (value) => {
		    return PBS.Utils.renderMaintenance(value);
		},
		flex: 1,
	    }
	],
    },
});
