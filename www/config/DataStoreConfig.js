Ext.define('pbs-datastore-list', {
    extend: 'Ext.data.Model',
    fields: ['name', 'comment'],
    proxy: {
        type: 'proxmox',
	url: "/api2/json/admin/datastore",
    },
    idProperty: 'store',
});

Ext.define('pbs-data-store-config', {
    extend: 'Ext.data.Model',
    fields: [
	'name', 'path', 'comment', 'gc-schedule', 'prune-schedule',
	'verify-schedule', 'keep-last', 'keep-hourly', 'keep-daily',
	'keep-weekly', 'keep-monthly', 'keep-yearly',
    ],
    proxy: {
        type: 'proxmox',
	url: "/api2/json/config/datastore",
    },
    idProperty: 'name',
});

Ext.define('PBS.DataStoreConfig', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsDataStoreConfig',

    title: gettext('Datastore Configuration'),

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

	editDataStore: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    let name = encodeURIComponent(selection[0].data.name);
	    Ext.create('PBS.DataStoreEdit', {
		name: name,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	onVerify: function() {
	    var view = this.getView();

	    let rec = view.selModel.getSelection()[0];
	    if (!(rec && rec.data)) return;
	    let data = rec.data;

	    Proxmox.Utils.API2Request({
		url: `/admin/datastore/${data.name}/verify`,
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
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    disabled: true,
	    handler: 'editDataStore',
	},
	// remove_btn
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Verify'),
	    disabled: true,
	    handler: 'onVerify',
	},
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
	    header: gettext('GC Schedule'),
	    sortable: false,
	    width: 120,
	    dataIndex: 'gc-schedule',
	},
	{
	    header: gettext('Prune Schedule'),
	    sortable: false,
	    width: 120,
	    dataIndex: 'prune-schedule',
	},
	{
	    header: gettext('Keep'),
	    columns: [
		{
		    text: gettext('Last'),
		    dataIndex: 'keep-last',
		    width: 70,
		},
		{
		    text: gettext('Hourly'),
		    dataIndex: 'keep-hourly',
		    width: 70,
		},
		{
		    text: gettext('Daily'),
		    dataIndex: 'keep-daily',
		    width: 70,
		},
		{
		    text: gettext('Weekly'),
		    dataIndex: 'keep-weekly',
		    width: 70,
		},
		{
		    text: gettext('Monthly'),
		    dataIndex: 'keep-monthly',
		    width: 70,
		},
		{
		    text: gettext('Yearly'),
		    dataIndex: 'keep-yearly',
		    width: 70,
		},
	    ],
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
	itemdblclick: 'editDataStore',
    },
});
