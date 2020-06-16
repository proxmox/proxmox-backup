Ext.define('PBS.admin.Directorylist', {
    extend: 'Ext.grid.Panel',
    xtype: 'pbsDirectoryList',

    stateful: true,
    stateId: 'grid-node-directory',

    emptyText: gettext('No Mount-Units found'),

    controller: {
	xclass: 'Ext.app.ViewController',

	createDirectory: function() {
	    let me = this;
	    Ext.create('PBS.window.CreateDirectory', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	reload: function() {
	    let me = this;
	    let store = me.getView().getStore();
	    store.load();
	    store.sort();
	},

	init: function(view) {
	    let me = this;
	    Proxmox.Utils.monStoreErrors(view, view.getStore(), true);
	    me.reload();
	},
    },


    rootVisible: false,
    useArrows: true,

    tbar: [
	{
	    text: gettext('Reload'),
	    iconCls: 'fa fa-refresh',
	    handler: 'reload',
	},
	{
	    text: gettext('Create') + ': Directory',
	    handler: 'createDirectory',
	},
    ],

    columns: [
	{
	    text: gettext('Path'),
	    dataIndex: 'path',
	    flex: 1,
	},
	{
	    header: gettext('Device'),
	    flex: 1,
	    dataIndex: 'device',
	},
	{
	    header: gettext('Filesystem'),
	    width: 100,
	    dataIndex: 'filesystem',
	},
	{
	    header: gettext('Options'),
	    width: 100,
	    dataIndex: 'options',
	},
	{
	    header: gettext('Unit File'),
	    hidden: true,
	    dataIndex: 'unitfile',
	},
    ],

    store: {
	fields: ['path', 'device', 'filesystem', 'options', 'unitfile'],
	proxy: {
	    type: 'proxmox',
	    url: '/api2/json/nodes/localhost/disks/directory',
	},
	sorters: 'path',
    },
});
