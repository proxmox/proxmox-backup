Ext.define('pbs-data-store-config', {
    extend: 'Ext.data.Model',
    fields: [ 'name', 'path', 'comment' ],
    proxy: {
        type: 'proxmox',
	url: "/api2/json/config/datastore"
    },
    idProperty: 'name'
});

Ext.define('PBS.DataStoreConfig', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsDataStoreConfig',

    initComponent : function() {
        var me = this;

	var store = new Ext.data.Store({
	    model: 'pbs-data-store-config',
	    sorters: 'name',
	});

	var reload = function() {
            store.load();
        };

 	var tbar = [
            {
		text: gettext('Create'),
		handler: function() {
		    alert("not implemented");
		    //var win = Ext.create('PVE.dc.PoolEdit', {});
		    //win.on('destroy', reload);
		    //win.show();
		}
            }
	    //edit_btn, remove_btn
        ];

	var sm = Ext.create('Ext.selection.RowModel', {});

	Proxmox.Utils.monStoreErrors(me, store);

        Ext.apply(me, {
            store: store,
            selModel: sm,
	    tbar: tbar,
            viewConfig: {
		trackOver: false
            },
            columns: [
                {
                    header: gettext('Name'),
		    sortable: true,
		    dataIndex: 'name',
		    flex: 1
		},
		{
                   header: gettext('Path'),
		   sortable: true,
		   dataIndex: 'path',
		    flex: 1
		},
		{
		    header: gettext('Comment'),
		    sortable: false,
		    dataIndex: 'comment',
		    renderer: Ext.String.htmlEncode,
		    flex: 2
		}
	    ],
	    listeners: {
                activate: reload
	    }
	});

	me.callParent();

	store.load();
    }
});
