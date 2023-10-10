Ext.define('PBS.admin.Directorylist', {
    extend: 'Ext.grid.Panel',
    xtype: 'pbsDirectoryList',

    stateful: true,
    stateId: 'grid-node-directory',

    emptyText: gettext('No Mount-Units found'),

    viewModel: {
	data: {
	    path: '',
	},
	formulas: {
	    dirName: (get) => get('path')?.replace('/mnt/datastore/', '') || undefined,
	},
    },

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

	removeDirectory: function() {
	    let me = this;
	    let vm = me.getViewModel();

	    let dirName = vm.get('dirName');

	    if (!dirName) {
		throw "no directory name specified";
	    }

	    Ext.create('Proxmox.window.SafeDestroy', {
		url: `/nodes/localhost/disks/directory/${dirName}`,
		item: { id: dirName },
		showProgress: true,
		taskName: 'dirremove',
		listeners: {
		    destroy: () => me.reload(),
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
	'->',
	{
	    xtype: 'tbtext',
	    data: {
		dirName: undefined,
	    },
	    bind: {
		data: {
		    dirName: "{dirName}",
		},
	    },
	    tpl: [
		'<tpl if="dirName">',
		gettext('Directory') + ' {dirName}:',
		'<tpl else>',
		Ext.String.format(gettext('No {0} selected'), gettext('directory')),
		'</tpl>',
	    ],
	},
	{
	    text: gettext('More'),
	    iconCls: 'fa fa-bars',
	    disabled: true,
	    bind: {
		disabled: '{!dirName}',
	    },
	    menu: [
		{
		    text: gettext('Remove'),
		    itemId: 'remove',
		    iconCls: 'fa fa-fw fa-trash-o',
		    handler: 'removeDirectory',
		    disabled: true,
		    bind: {
			disabled: '{!dirName}',
		    },
		},
	    ],
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

    listeners: {
	activate: "reload",
	selectionchange: function(model, selected) {
	    let me = this;
	    let vm = me.getViewModel();

	    vm.set('path', selected[0]?.data.path || '');
	},
    },

    store: {
	fields: ['path', 'device', 'filesystem', 'options', 'unitfile'],
	proxy: {
	    type: 'proxmox',
	    url: '/api2/json/nodes/localhost/disks/directory',
	},
	sorters: 'path',
    },
});
