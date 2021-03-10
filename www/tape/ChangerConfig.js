Ext.define('pbs-model-changers', {
    extend: 'Ext.data.Model',
    fields: ['path', 'model', 'name', 'serial', 'vendor', 'export-slots'],
    idProperty: 'name',
});

Ext.define('PBS.TapeManagement.ChangerPanel', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsTapeChangerPanel',

    controller: {
	xclass: 'Ext.app.ViewController',

	reloadTapeStore: function() {
	    let navtree = Ext.ComponentQuery.query('navigationtree')[0];
	    navtree.reloadTapeStore();
	},

	onAdd: function() {
	    let me = this;
	    Ext.create('PBS.TapeManagement.ChangerEditWindow', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	onEdit: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }
	    Ext.create('PBS.TapeManagement.ChangerEditWindow', {
		changerid: selection[0].data.name,
		autoLoad: true,
		listeners: {
		    destroy: () => me.reload(),
		},
	    }).show();
	},

	reload: function() {
	    this.getView().getStore().rstore.load();
	    this.reloadTapeStore();
	},

	stopStore: function() {
	    this.getView().getStore().rstore.stopUpdate();
	},

	startStore: function() {
	    this.getView().getStore().rstore.startUpdate();
	},

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	beforedestroy: 'stopStore',
	deactivate: 'stopStore',
	activate: 'startStore',
	itemdblclick: 'onEdit',
    },

    store: {
	type: 'diff',
	rstore: {
	    type: 'update',
	    storeid: 'proxmox-tape-changers',
	    model: 'pbs-model-changers',
	    proxy: {
		type: 'proxmox',
		url: "/api2/json/tape/changer",
	    },
	},
	sorters: 'name',
    },

    tbar: [
	{
	    text: gettext('Add'),
	    xtype: 'proxmoxButton',
	    handler: 'onAdd',
	    selModel: false,
	},
	'-',
	{
	    text: gettext('Edit'),
	    xtype: 'proxmoxButton',
	    handler: 'onEdit',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/api2/extjs/config/changer',
	    callback: 'reload',
	},
    ],
    columns: [
	{
	    text: gettext('Name'),
	    dataIndex: 'name',
	    flex: 1,
	},
	{
	    text: gettext('Path'),
	    dataIndex: 'path',
	    flex: 2,
	},
	{
	    text: gettext('Vendor'),
	    dataIndex: 'vendor',
	    flex: 1,
	},
	{
	    text: gettext('Model'),
	    dataIndex: 'model',
	    flex: 1,
	},
	{
	    text: gettext('Serial'),
	    dataIndex: 'serial',
	    flex: 1,
	},
	{
	    text: gettext('Import/Export Slots'),
	    dataIndex: 'export-slots',
	    flex: 1,
	},
    ],
});

