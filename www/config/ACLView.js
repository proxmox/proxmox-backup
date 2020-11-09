Ext.define('pmx-acls', {
    extend: 'Ext.data.Model',
    fields: [
	'path', 'ugid', 'ugid_type', 'roleid', 'propagate',
	{
	    name: 'aclid',
	    calculate: function(data) {
		return `${data.path} for ${data.ugid} - ${data.roleid}`;
	    },
	},
    ],
    idProperty: 'aclid',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/access/acl',
    },
});

Ext.define('PBS.config.ACLView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsACLView',

    title: gettext('Permissions'),

    aclPath: undefined,
    aclExact: undefined,

    controller: {
	xclass: 'Ext.app.ViewController',

	addUserACL: function() {
	    let me = this;
	    let view = me.getView();
	    Ext.create('PBS.window.ACLEdit', {
		path: view.aclPath,
		aclType: 'user',
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	addTokenACL: function() {
	    let me = this;
	    let view = me.getView();
	    Ext.create('PBS.window.ACLEdit', {
		path: view.aclPath,
		aclType: 'token',
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},


	removeACL: function(btn, event, rec) {
	    let me = this;
	    Proxmox.Utils.API2Request({
		url: '/access/acl',
		method: 'PUT',
		params: {
		    'delete': 1,
		    path: rec.data.path,
		    role: rec.data.roleid,
		    'auth-id': rec.data.ugid,
		},
		callback: function() {
		    me.reload();
		},
		failure: function(response, opts) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
	    });
	},

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    let proxy = view.getStore().rstore.getProxy();

	    let params = {};
	    if (view.aclPath !== undefined) {

		let pathFilter = Ext.create('Ext.util.Filter', {
		    filterPath: view.aclPath,
		    filterFn: function(item) {
			let me = this;
			let curr = item.data.path;

			if (curr.lastIndexOf("/") < me.filterPath.lastIndexOf("/")) {
			    return me.filterPath.startsWith(curr);
			} else {
			    return me.filterPath === curr;
			}
		    },
		});
		view.getStore().addFilter(pathFilter);
	    }
	    if (view.aclExact !== undefined) {
		if (view.aclPath !== undefined) {
		    params.path = view.aclPath;
		}
		params.exact = view.aclExact;
	    }

	    proxy.setExtraParams(params);
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
	control: {
	    '#': { // view
		activate: function() {
		    this.getView().getStore().rstore.startUpdate();
		},
		deactivate: function() {
		    this.getView().getStore().rstore.stopUpdate();
		},
	    },
	},
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'aclid',
	rstore: {
	    type: 'update',
	    storeid: 'pmx-acls',
	    model: 'pmx-acls',
	    interval: 5000,
	},
    },

    tbar: [
	{
	    text: gettext('Add'),
	    menu: {
		xtype: 'menu',
		items: [
		    {
			text: gettext('User Permission'),
			iconCls: 'fa fa-fw fa-user',
			handler: 'addUserACL',
		    },
		    {
			text: gettext('API Token Permission'),
			iconCls: 'fa fa-fw fa-user-o',
			handler: 'addTokenACL',
		    },
		],
	    },
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    handler: 'removeACL',
	    callback: 'reload',
	},
    ],

    columns: [
	{
	    header: gettext('Path'),
	    width: 250,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'path',
	},
	{
	    header: gettext('User/Group/API Token'),
	    width: 200,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'ugid',
	},
	{
	    header: gettext('Role'),
	    width: 200,
	    sortable: true,
	    dataIndex: 'roleid',
	},
	{
	    header: gettext('Propagate'),
	    flex: 1, // last element flex looks better
	    sortable: true,
	    renderer: Proxmox.Utils.format_boolean,
	    dataIndex: 'propagate',
	},
    ],
});
