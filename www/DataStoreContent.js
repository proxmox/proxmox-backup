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

    store: {
	model: 'pbs-data-store-content',
	sorters: 'name',
    },

    reload: function() {
	let url = `/api2/json/admin/datastore/${this.datastore}/snapshots`;
	this.store.setProxy({
	    type: 'proxmox',
	    url: url
	});
	this.store.load();
    },

    initComponent : function() {
        var me = this;

	if (!me.datastore) {
	    throw "no datastore specified";
	}

	me.title = gettext('Data Store Content: ') + me.datastore;

	me.callParent();

	Proxmox.Utils.monStoreErrors(me, me.store, true);
	me.reload(); // initial load
    }
});
