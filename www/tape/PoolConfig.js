Ext.define('pbs-model-media-pool', {
    extend: 'Ext.data.Model',
    fields: [
	'name', 'allocation', 'retention', 'template', 'encrypt', 'comment',
	{
	    name: 'encryption',
	    type: 'bool',
	    calculate: function(data) {
		return !!data.encrypt;
	    },
	},
    ],
    idProperty: 'name',
});

Ext.define('PBS.TapeManagement.PoolPanel', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsMediaPoolPanel',

    controller: {
	xclass: 'Ext.app.ViewController',

	onAdd: function() {
	    let me = this;
	    Ext.create('PBS.TapeManagement.PoolEditWindow', {
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
	    Ext.create('PBS.TapeManagement.PoolEditWindow', {
		poolid: selection[0].data.name,
		autoLoad: true,
		listeners: {
		    destroy: () => me.reload(),
		},
	    }).show();
	},

	reload: function() {
	    this.getView().getStore().rstore.load();
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
	    storeid: 'proxmox-tape-media-pools',
	    model: 'pbs-model-media-pool',
	    proxy: {
		type: 'proxmox',
		url: "/api2/json/config/media-pool",
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
	    baseurl: '/api2/extjs/config/media-pool',
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
	    text: gettext('Allocation Policy'),
	    dataIndex: 'allocation',
	},
	{
	    text: gettext('Retention Policy'),
	    dataIndex: 'retention',
	},
	{
	    text: gettext('Encryption'),
	    dataIndex: 'encryption',
	    renderer: Proxmox.Utils.format_boolean,
	},
	{
	    text: gettext('Encryption Fingerprint'),
	    dataIndex: 'encrypt',
	    hidden: true,
	    flex: 3,
	},
	{
	    text: gettext('Comment'),
	    dataIndex: 'comment',
	    flex: 3,
	},
    ],
});
