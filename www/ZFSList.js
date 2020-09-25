Ext.define('PBS.admin.ZFSList', {
    extend: 'Ext.grid.Panel',
    xtype: 'pbsZFSList',

    stateful: true,
    stateId: 'grid-node-zfs',

    controller: {
	xclass: 'Ext.app.ViewController',

	openCreateWindow: function() {
	    let me = this;
	    Ext.create('PBS.window.CreateZFS', {
		nodename: me.nodename,
		listeners: {
		    destroy: function() { me.reload(); },
		},
	    }).show();
	},

	openDetailWindow: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) return;

	    let rec = selection[0];
	    let zpool = rec.get('name');

	    Ext.create('Proxmox.window.ZFSDetail', {
		zpool,
		nodename: view.nodename,
	    }).show();
	},

	reload: function() {
	    let me = this;
	    let view = me.getView();
	    let store = view.getStore();
	    store.load();
	    store.sort();
	},

	init: function(view) {
	    let me = this;

	    if (!view.nodename) {
		throw "no nodename given";
	    }

	    let url = `/api2/json/nodes/${view.nodename}/disks/zfs`;
	    view.getStore().getProxy().setUrl(url);

	    Proxmox.Utils.monStoreErrors(view, view.getStore(), true);

	    me.reload();
	},
    },

    columns: [
	{
	    text: gettext('Name'),
	    dataIndex: 'name',
	    flex: 1,
	},
	{
	    header: gettext('Size'),
	    renderer: Proxmox.Utils.format_size,
	    dataIndex: 'size',
	},
	{
	    header: gettext('Free'),
	    renderer: Proxmox.Utils.format_size,
	    dataIndex: 'free',
	},
	{
	    header: gettext('Allocated'),
	    renderer: Proxmox.Utils.format_size,
	    dataIndex: 'alloc',
	},
	{
	    header: gettext('Fragmentation'),
	    renderer: function(value) {
		return value.toString() + '%';
	    },
	    dataIndex: 'frag',
	},
	{
	    header: gettext('Health'),
	    renderer: Proxmox.Utils.render_zfs_health,
	    dataIndex: 'health',
	},
	{
	    header: gettext('Deduplication'),
	    hidden: true,
	    renderer: function(value) {
		return value.toFixed(2).toString() + 'x';
	    },
	    dataIndex: 'dedup',
	},
    ],

    rootVisible: false,
    useArrows: true,

    tbar: [
	{
	    text: gettext('Reload'),
	    iconCls: 'fa fa-refresh',
	    handler: 'reload',
	},
	{
	    text: gettext('Create') + ': ZFS',
	    handler: 'openCreateWindow',
	},
	{
	    text: gettext('Detail'),
	    xtype: 'proxmoxButton',
	    disabled: true,
	    handler: 'openDetailWindow',
	},
    ],

    listeners: {
	itemdblclick: 'openDetailWindow',
    },

    store: {
	fields: ['name', 'size', 'free', 'alloc', 'dedup', 'frag', 'health'],
	proxy: {
	    type: 'proxmox',
	},
	sorters: 'name',
    },
});

