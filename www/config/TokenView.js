Ext.define('pbs-tokens', {
    extend: 'Ext.data.Model',
    fields: [
	'tokenid', 'tokenname', 'user', 'comment',
	{ type: 'boolean', name: 'enable', defaultValue: true },
	{ type: 'date', dateFormat: 'timestamp', name: 'expire' },
    ],
    idProperty: 'tokenid',
});

Ext.define('pbs-users-with-tokens', {
    extend: 'Ext.data.Model',
    fields: [
	'userid', 'firstname', 'lastname', 'email', 'comment',
	{ type: 'boolean', name: 'enable', defaultValue: true },
	{ type: 'date', dateFormat: 'timestamp', name: 'expire' },
	'tokens',
    ],
    idProperty: 'userid',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/access/users/?include_tokens=1',
    },
});

Ext.define('PBS.config.TokenView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsTokenView',

    stateful: true,
    stateId: 'grid-tokens',

    title: gettext('API Tokens'),

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    view.userStore = Ext.create('Proxmox.data.UpdateStore', {
		autoStart: true,
		interval: 5 * 1000,
		storeId: 'pbs-users-with-tokens',
		storeid: 'pbs-users-with-tokens',
		model: 'pbs-users-with-tokens',
	    });
	    view.userStore.on('load', this.onLoad, this);
	    view.on('destroy', view.userStore.stopUpdate);
	    Proxmox.Utils.monStoreErrors(view, view.userStore);
	},

	reload: function() { this.getView().userStore.load(); },

	onLoad: function(store, data, success) {
	    if (!success) return;

	    let tokenStore = this.getView().store.rstore;

	    let records = [];
	    Ext.Array.each(data, function(user) {
		let tokens = user.data.tokens || [];
		Ext.Array.each(tokens, function(token) {
		    let r = {};
		    r.tokenid = token.tokenid;
		    r.comment = token.comment;
		    r.expire = token.expire;
		    r.enable = token.enable;
		    records.push(r);
		});
	    });

	    tokenStore.loadData(records);
	    tokenStore.fireEvent('load', tokenStore, records, true);
	},

	addToken: function() {
	    let me = this;
	    Ext.create('PBS.window.TokenEdit', {
		isCreate: true,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	editToken: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;
	    Ext.create('PBS.window.TokenEdit', {
		user: PBS.Utils.extractTokenUser(selection[0].data.tokenid),
		tokenname: PBS.Utils.extractTokenName(selection[0].data.tokenid),
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	showPermissions: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();

	    if (selection.length < 1) return;

	    Ext.create('Proxmox.PermissionView', {
		auth_id: selection[0].data.tokenid,
		auth_id_name: 'auth-id',
	    }).show();
	},

	renderUser: function(tokenid) {
	    return Ext.String.htmlEncode(PBS.Utils.extractTokenUser(tokenid));
	},

	renderTokenname: function(tokenid) {
	    return Ext.String.htmlEncode(PBS.Utils.extractTokenName(tokenid));
	},

    },

    listeners: {
	activate: 'reload',
	itemdblclick: 'editToken',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'tokenid',
	model: 'pbs-tokens',
	rstore: {
	    type: 'store',
	    proxy: 'memory',
	    storeid: 'pbs-tokens',
	    model: 'pbs-tokens',
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addToken',
	    selModel: false,
	},
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editToken',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/access/users/',
	    callback: 'reload',
	    getUrl: function(rec) {
		let tokenid = rec.getId();
		let user = PBS.Utils.extractTokenUser(tokenid);
		let tokenname = PBS.Utils.extractTokenName(tokenid);
		return '/access/users/' + encodeURIComponent(user) + '/token/' + encodeURIComponent(tokenname);
	    },
	},
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Show Permissions'),
	    handler: 'showPermissions',
	    disabled: true,
	},
    ],

    viewConfig: {
	trackOver: false,
    },

    columns: [
	{
	    header: gettext('User'),
	    width: 200,
	    sortable: true,
	    renderer: 'renderUser',
	    dataIndex: 'tokenid',
	},
	{
	    header: gettext('Token name'),
	    width: 100,
	    sortable: true,
	    renderer: 'renderTokenname',
	    dataIndex: 'tokenid',
	},
	{
	    header: gettext('Enabled'),
	    width: 80,
	    sortable: true,
	    renderer: Proxmox.Utils.format_boolean,
	    dataIndex: 'enable',
	},
	{
	    header: gettext('Expire'),
	    width: 80,
	    sortable: true,
	    renderer: Proxmox.Utils.format_expire,
	    dataIndex: 'expire',
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
