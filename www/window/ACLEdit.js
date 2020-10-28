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

    initComponent: function() {
	let me = this;

	me.items = [];

	me.items.push({
	    xtype: 'pbsPermissionPathSelector',
	    fieldLabel: gettext('Path'),
	    editable: !me.path,
	    value: me.path,
	    name: 'path',
	    allowBlank: false,
	});

	if (me.aclType === 'user') {
	    me.subject = gettext('User Permission');
	    me.items.push({
		xtype: 'pbsUserSelector',
		fieldLabel: gettext('User'),
		name: 'auth_id',
		allowBlank: false,
	    });
	} else if (me.aclType === 'token') {
	    me.subject = gettext('API Token Permission');
	    me.items.push({
		xtype: 'pbsTokenSelector',
		fieldLabel: gettext('API Token'),
		name: 'auth_id',
		allowBlank: false,
	    });
	}
	me.items.push({
	    xtype: 'pmxRoleSelector',
	    name: 'role',
	    value: 'NoAccess',
	    fieldLabel: gettext('Role'),
	});
	me.items.push({
	    xtype: 'proxmoxcheckbox',
	    name: 'propagate',
	    checked: true,
	    uncheckedValue: 0,
	    fieldLabel: gettext('Propagate'),
	});

	me.callParent();
    },

    getValues: function(dirtyOnly) {
	let me = this;
	let values = me.callParent(arguments);

	if (me.path) {
	    values.path = me.path;
	}
	return values;
    },

});
