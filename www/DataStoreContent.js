Ext.define('pbs-data-store-content', {
    extend: 'Ext.data.Model',
    fields: [ 'snapshot' ],
});

Ext.define('PBS.DataStoreContent', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsDataStoreContent',

    initComponent : function() {
        var me = this;

	if (!me.datastore) {
	    throw "no datastore specified";
	}

	me.title =  gettext('Data Store Content: ') + me.datastore;

	var store = new Ext.data.Store({
	    model: 'pbs-data-store-content',
	    sorters: 'name',
	});

	var reload = function() {
	    var url = '/api2/json/admin/datastore/' + me.datastore + '/snapshots';
	    me.store.setProxy({
		type: 'proxmox',
		url: url
	    });
            me.store.load();
        };


	Ext.apply(me, {
	    store: store,
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
		    flex: 1
		}
	    ],
	});

	me.callParent();

	reload();
    }
});
