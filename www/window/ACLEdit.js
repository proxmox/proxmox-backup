Ext.define('PBS.window.ACLEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsACLAdd',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_acl',

    url: '/access/acl',
    method: 'PUT',
    isAdd: true,
    isCreate: true,
    width: 450,

    // caller can give a static path
    path: undefined,
    defaultFocus: 'proxmoxcheckbox',

    initComponent: function() {
	let me = this;

	me.items = [];

	me.items.push({
	    xtype: 'pbsPermissionPathSelector',
	    name: 'path',
	    fieldLabel: gettext('Path'),
	    allowBlank: false,
	    //editable: !me.path,
	    value: me.path,
	    datastore: me.datastore,
	});

	if (me.aclType === 'user') {
	    me.subject = gettext('User Permission');
	    me.items.push({
		xtype: 'pmxUserSelector',
		name: 'auth-id',
		fieldLabel: gettext('User'),
		allowBlank: false,
	    });
	} else if (me.aclType === 'token') {
	    me.subject = gettext('API Token Permission');
	    me.items.push({
		xtype: 'pbsTokenSelector',
		name: 'auth-id',
		fieldLabel: gettext('API Token'),
		allowBlank: false,
	    });
	}
	me.items.push({
	    xtype: 'pmxRoleSelector',
	    name: 'role',
	    fieldLabel: gettext('Role'),
	    value: 'NoAccess',
	});
	me.items.push({
	    xtype: 'proxmoxcheckbox',
	    name: 'propagate',
	    fieldLabel: gettext('Propagate'),
	    checked: true,
	    uncheckedValue: 0,
	});

	me.callParent();
    },
});
