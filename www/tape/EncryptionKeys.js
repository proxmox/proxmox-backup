Ext.define('pbs-tape-encryption-keys', {
    extend: 'Ext.data.Model',
    fields: [
	'fingerprint', 'hint', 'kdf', 'modified',
	{
	    name: 'created',
	    type: 'date',
	    dateFormat: 'timestamp',
	},
    ],
    idProperty: 'fingerprint',
});

Ext.define('PBS.TapeManagement.EncryptionPanel', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsEncryptionKeys',

    controller: {
	xclass: 'Ext.app.ViewController',

	onAdd: function() {
	    let me = this;
	    Ext.create('PBS.TapeManagement.EncryptionEditWindow', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
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
    },

    listeners: {
	beforedestroy: 'stopStore',
	deactivate: 'stopStore',
	activate: 'startStore',
    },

    store: {
	type: 'diff',
	rstore: {
	    type: 'update',
	    storeid: 'proxmox-tape-encryption-keys',
	    model: 'pbs-tape-encryption-keys',
	    proxy: {
		type: 'proxmox',
		url: "/api2/json/config/tape-encryption-keys",
	    },
	},
	sorters: 'hint',
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
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/api2/extjs/config/tape-encryption-keys',
	    callback: 'reload',
	},
    ],
    columns: [
	{
	    text: gettext('Hint'),
	    dataIndex: 'hint',
	    flex: 1,
	},
	{
	    text: gettext('Fingerprint'),
	    dataIndex: 'fingerprint',
	    flex: 4,
	},
	{
	    text: gettext('Created'),
	    dataIndex: 'created',
	    flex: 2,
	},
    ],
});
