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

	onRestoreKey: function() {
	    Ext.create('Proxmox.window.Edit', {
		title: gettext('Restore Key'),
		isCreate: true,
		submitText: gettext('Restore'),
		method: 'POST',
		url: `/api2/extjs/tape/drive`,
		submitUrl: function(url, values) {
		    let drive = values.drive;
		    delete values.drive;
		    return `${url}/${drive}/restore-key`;
		},
		items: [
		    {
			xtype: 'pbsDriveSelector',
			fieldLabel: gettext('Drive'),
			name: 'drive',
		    },
		    {
			xtype: 'textfield',
			inputType: 'password',
			fieldLabel: gettext('Password'),
			name: 'password',
		    },
		],
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
	'-',
	{
	    text: gettext('Restore Key'),
	    xtype: 'proxmoxButton',
	    handler: 'onRestoreKey',
	    selModel: false,
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
	    xtype: 'datecolumn',
	    dataIndex: 'created',
	    width: 150,
	    format: 'Y-m-d H:i:s',
	},
    ],
});
