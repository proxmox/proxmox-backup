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

    controller: {
	xclass: 'Ext.app.ViewController',

	createDataStore: function() {
	    let me = this;
	    Ext.create('PBS.DataStoreEdit', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	garbageCollect: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    let name = encodeURIComponent(selection[0].data.name);
	    Proxmox.Utils.API2Request({
		url: `/admin/datastore/${name}/gc`,
		method: 'POST',
		failure: function(response) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
		success: function(response, options) {
		    Ext.create('Proxmox.window.TaskViewer', {
			upid: response.result.data,
		    }).show();
		},
	    });
	},

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'name',
	rstore: {
	    type: 'update',
	    storeid: 'pbs-data-store-config',
	    model: 'pbs-data-store-config',
	    autoStart: true,
	    interval: 10000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    selModel: false,
	    text: gettext('Create'),
	    handler: 'createDataStore',
	},
	// edit/remove button
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Start GC'),
	    disabled: true,
	    handler: 'garbageCollect',
	},
    ],

    columns: [
	{
	    header: gettext('Name'),
	    sortable: true,
	    dataIndex: 'name',
	    flex: 1,
	},
	{
	    header: gettext('Path'),
	    sortable: true,
	    dataIndex: 'path',
	    flex: 1,
	},
	{
	    header: gettext('Comment'),
	    sortable: false,
	    dataIndex: 'comment',
	    renderer: Ext.String.htmlEncode,
	    flex: 2,
	},
    ],

    listeners: {
	activate: 'reload',
    },
});
