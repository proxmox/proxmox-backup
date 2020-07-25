Ext.define('PBS.window.ACLEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsACLAdd',
    mixins: ['Proxmox.Mixin.CBind'],

    url: '/access/acl',
    method: 'PUT',
    isAdd: true,
    isCreate: true,
    width: 450,

    // caller can give a static path
    path: undefined,

    subject: gettext('User Permission'),

    getValues: function(dirtyOnly) {
	let me = this;
	let values = me.callParent(arguments);

	if (me.path) {
	    values.path = me.path;
	}
	return values;
    },

    items: [
	{
	    xtype: 'pbsPermissionPathSelector',
	    fieldLabel: gettext('Path'),
	    cbind: {
		editable: '{!path}',
		value: '{path}',
	    },
	    name: 'path',
	    allowBlank: false,
	},
	{
	    xtype: 'pbsUserSelector',
	    fieldLabel: gettext('User'),
	    name: 'userid',
	    allowBlank: false,
	},
	{
	    xtype: 'pmxRoleSelector',
	    name: 'role',
	    value: 'NoAccess',
	    fieldLabel: gettext('Role'),
	},
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'propagate',
	    checked: true,
	    uncheckedValue: 0,
	    fieldLabel: gettext('Propagate'),
	},
    ],
});
