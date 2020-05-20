Ext.define('pmx-acls', {
    extend: 'Ext.data.Model',
    fields: [
	'path', 'ugid', 'ugid_type', 'roleid', 'propagate',
	{
	    name: 'aclid',
	    calculate: function(data) {
		return `${data.path} for ${data.ugid}`;
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

    stateful: true,
    stateId: 'grid-acls',

    title: gettext('ACLs'),


    aclPath: undefined,
    aclExact: undefined,

    controller: {
	xclass: 'Ext.app.ViewController',

	addACL: function() {
	    let me = this;
	    let view = me.getView();
            Ext.create('PBS.window.ACLEdit', {
		path: view.aclPath,
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
		url:'/access/acl',
		method: 'PUT',
		params: {
		    'delete': 1,
		    path: rec.data.path,
		    role: rec.data.roleid,
		    userid: rec.data.ugid,
		},
		callback: function() {
		    me.reload();
		},
		failure: function (response, opts) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
	    });
	},

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    let proxy = view.getStore().rstore.getProxy();

	    let params = {};
	    if (view.aclPath !== undefined) {
		params.path = view.aclPath;
	    }
	    if (view.aclExact !== undefined) {
		params.exact = view.aclExact;
	    }
	    proxy.setExtraParams(params);
	},
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'userid',
	rstore: {
	    type: 'update',
	    storeid: 'pmx-acls',
	    model: 'pmx-acls',
	    autoStart: true,
	    interval: 5000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addACL',
	    selModel: false,
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
	    width: 200,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'path',
	},
	{
	    header: gettext('User/Group'),
	    width: 100,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'ugid',
	},
	{
	    header: gettext('Role'),
	    width: 80,
	    sortable: true,
	    dataIndex: 'roleid',
	},
	{
	    header: gettext('Propagate'),
	    width: 150,
	    sortable: true,
	    renderer: Proxmox.Utils.format_boolean,
	    dataIndex: 'propagate',
	},
    ],

});
