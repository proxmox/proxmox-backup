Ext.define('pmx-remotes', {
    extend: 'Ext.data.Model',
    fields: [ 'name', 'host', 'userid', 'fingerprint', 'comment' ],
    idProperty: 'name',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/config/remote',
    },
});

Ext.define('PBS.config.RemoteView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsRemoteView',

    stateful: true,
    stateId: 'grid-remotes',

    title: gettext('Remotes'),

    controller: {
	xclass: 'Ext.app.ViewController',

	addRemote: function() {
	    let me = this;
            Ext.create('PBS.window.RemoteEdit', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
            }).show();
	},

	editRemote: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

            Ext.create('PBS.window.RemoteEdit', {
                name: selection[0].data.name,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
            }).show();
	},

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	activate: 'reload',
	itemdblclick: 'editRemote',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'name',
	rstore: {
	    type: 'update',
	    storeid: 'pmx-remotes',
	    model: 'pmx-remotes',
	    autoStart: true,
	    interval: 5000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addRemote',
	    selModel: false,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editRemote',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/config/remote',
	    callback: 'reload',
	},
    ],

    viewConfig: {
	trackOver: false,
    },

    columns: [
	{
	    header: gettext('Remote'),
	    width: 200,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'name',
	},
	{
	    header: gettext('Host'),
	    width: 200,
	    sortable: true,
	    dataIndex: 'host',
	},
	{
	    header: gettext('User name'),
	    width: 200,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'userid',
	},
	{
	    header: gettext('Fingerprint'),
	    sortable: false,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'fingerprint',
	    width: 200,
	},
	{
	    header: gettext('Comment'),
	    sortable: false,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'comment',
	    flex: 1,
	},
    ],
});
