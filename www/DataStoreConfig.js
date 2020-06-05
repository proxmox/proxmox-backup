Ext.define('pbs-datastore-list', {
    extend: 'Ext.data.Model',
    fields: [ 'name', 'comment' ],
    proxy: {
        type: 'proxmox',
	url: "/api2/json/admin/datastore"
    },
    idProperty: 'store'
});

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

    title: gettext('Data Store Configuration'),

    initComponent : function() {
        var me = this;

	var store = new Ext.data.Store({
	    model: 'pbs-data-store-config',
	    sorters: 'name',
	});

	var reload = function() {
            store.load();
        };

	var sm = Ext.create('Ext.selection.RowModel', {});

	var gc_btn = new Proxmox.button.Button({
	    text: gettext('Start GC'),
	    disabled: true,
	    selModel: sm,
	    handler: function() {
		var rec = sm.getSelection()[0];
		Proxmox.Utils.API2Request({
		    url: '/admin/datastore/' + rec.data.name + '/gc',
		    method: 'POST',
		    failure: function(response) {
			Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		    },
		    success: function(response, options) {
			var upid = response.result.data;

			var win = Ext.create('Proxmox.window.TaskViewer', {
			    upid: upid
			});
			win.show();
		    }
		});
	    }
	});

	var tbar = [
            {
		text: gettext('Create'),
		handler: function() {
		    let win = Ext.create('PBS.DataStoreEdit', {});
		    win.on('destroy', reload);
		    win.show();
		}
            },
	    '-',
	    gc_btn
	    //edit_btn, remove_btn
        ];

	Proxmox.Utils.monStoreErrors(me, store);

	Ext.apply(me, {
	    store: store,
	    selModel: sm,
	    tbar: tbar,
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

Ext.define('PBS.DataStoreInputPanel', {
    extend: 'Proxmox.panel.InputPanel',
    alias: 'widget.pbsDataStoreInputPanel',

    onGetValues: function(values) {
	var me = this;

	return values;
    },

    column1: [
	{
	    xtype: 'textfield',
	    name: 'name',
	    allowBlank: false,
	    fieldLabel: gettext('Name'),
	},
    ],

    column2: [
	{
	    xtype: 'textfield',
	    name: 'path',
	    allowBlank: false,
	    fieldLabel: gettext('Backing Path'),
	    emptyText: gettext('An absolute path'),
	},
    ],

    columnB: [
	{
	    xtype: 'textfield',
	    name: 'comment',
	    fieldLabel: gettext('Comment'),
	},
    ],
});

Ext.define('PBS.DataStoreEdit', {
    extend: 'Proxmox.window.Edit',

    url: '/api2/extjs/config/datastore',
    method: 'POST',

    subject: gettext('Datastore'),
    isAdd: true,
    items: [{
	xtype: 'pbsDataStoreInputPanel',
    }],
});
