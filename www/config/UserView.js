Ext.define('pmx-users', {
    extend: 'Ext.data.Model',
    fields: [
	'userid', 'firstname', 'lastname', 'email', 'comment',
	{ type: 'boolean', name: 'enable', defaultValue: true },
	{ type: 'date', dateFormat: 'timestamp', name: 'expire' },
    ],
    idProperty: 'userid',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/access/users',
    },
});

Ext.define('PBS.config.UserView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsUserView',

    stateful: true,
    stateId: 'grid-users',

    title: gettext('Users'),

    controller: {
	xclass: 'Ext.app.ViewController',

	addUser: function() {
	    let me = this;
            Ext.create('PBS.window.UserEdit', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
            }).show();
	},

	editUser: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

            Ext.create('PBS.window.UserEdit', {
                userid: selection[0].data.userid,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
            }).show();
	},

	setPassword: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();

	    if (selection.length < 1) return;

	    Ext.create('PBS.window.UserPassword', {
		url: '/api2/extjs/access/users/' + selection[0].data.userid,
	    }).show();
	},

	showPermissions: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();

	    if (selection.length < 1) return;

	    Ext.create('Proxmox.PermissionView', {
		auth_id: selection[0].data.userid,
		auth_id_name: 'auth_id',
	    }).show();
	},

	renderUsername: function(userid) {
	    return Ext.String.htmlEncode(userid.match(/^(.+)@([^@]+)$/)[1]);
	},

	renderRealm: function(userid) {
	    return Ext.String.htmlEncode(userid.match(/^(.+)@([^@]+)$/)[2]);
	},

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	activate: 'reload',
	itemdblclick: 'editUser',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'userid',
	rstore: {
	    type: 'update',
	    storeid: 'pmx-users',
	    model: 'pmx-users',
	    autoStart: true,
	    interval: 5000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addUser',
	    selModel: false,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editUser',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Password'),
	    handler: 'setPassword',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/access/users/',
	    enableFn: (rec) => rec.data.userid !== 'root@pam',
	    callback: 'reload',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Permissions'),
	    handler: 'showPermissions',
	    disabled: true,
	},
    ],

    viewConfig: {
	trackOver: false,
    },

    columns: [
	{
	    header: gettext('User name'),
	    width: 200,
	    sortable: true,
	    renderer: 'renderUsername',
	    dataIndex: 'userid',
	},
	{
	    header: gettext('Realm'),
	    width: 100,
	    sortable: true,
	    renderer: 'renderRealm',
	    dataIndex: 'userid',
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
	    header: gettext('Name'),
	    width: 150,
	    sortable: true,
	    dataIndex: 'firstname',
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
