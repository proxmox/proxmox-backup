Ext.define('pbs-data-store-content', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-id',
	'backup-time',
	'backup-type',
	'files',
	{ name: 'size', type: 'int', defaultValue: 0 },
    ],
});

Ext.define('PBS.DataStoreContent', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsDataStoreContent',

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    if (!view.datastore) {
		throw "no datastore specified";
	    }

	    view.title = gettext('Data Store Content: ') + view.datastore;

	    Proxmox.Utils.monStoreErrors(view, view.store, true);
	    this.reload(); // initial load
	},

	reload: function() {
	    var view = this.getView();

	    let url = `/api2/json/admin/datastore/${view.datastore}/snapshots`;
	    view.store.setProxy({
		type: 'proxmox',
		url:  url
	    });
	    view.store.load();
	},
    },

    columns: [
	{
	    header: gettext('Type'),
	    sortable: true,
	    dataIndex: 'backup-type',
	    flex: 1
	},
	{
	    header: gettext('ID'),
	    sortable: true,
	    dataIndex: 'backup-id',
	    flex: 1
	},
	{
	    header: gettext('Time'),
	    sortable: true,
	    dataIndex: 'backup-time',
	    renderer: Proxmox.Utils.render_timestamp,
	    flex: 1
	},
	{
	    header: gettext('Size'),
	    sortable: true,
	    dataIndex: 'size',
	    renderer: Proxmox.Utils.format_size,
	    flex: 1
	},
    ],

    tbar: [
	{
	    text: gettext('Reload'),
	    iconCls: 'fa fa-refresh',
	    handler: 'reload',
	},
    ],

    store: {
	model: 'pbs-data-store-content',
	sorters: 'name',
    },
});
