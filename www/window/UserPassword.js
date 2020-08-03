Ext.define('PBS.window.UserPassword', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsUserPassword',

    userid: undefined,

    method: 'PUT',

    subject: gettext('User Password'),

    fieldDefaults: { labelWidth: 120 },

    items: [
	{
	    xtype: 'textfield',
	    inputType: 'password',
	    fieldLabel: gettext('Password'),
	    minLength: 5,
	    allowBlank: false,
	    name: 'password',
	    listeners: {
		change: function(field) {
		    field.next().validate();
		},
		blur: function(field) {
		    field.next().validate();
		},
	    },
	},
	{
	    xtype: 'textfield',
	    inputType: 'password',
	    fieldLabel: gettext('Confirm password'),
	    name: 'verifypassword',
	    vtype: 'password',
	    initialPassField: 'password',
	    allowBlank: false,
	    submitValue: false,
	},
    ],
});
