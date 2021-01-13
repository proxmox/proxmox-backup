Ext.define('PBS.window.TfaEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsTfaEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_mgmt',

    modal: true,
    resizable: false,
    title: gettext("Modify a TFA entry's description"),
    width: 512,

    layout: {
	type: 'vbox',
	align: 'stretch',
    },

    cbindData: function(initialConfig) {
	let me = this;

	let tfa_id = initialConfig['tfa-id'];
	me.tfa_id = tfa_id;
	me.defaultFocus = 'textfield[name=description]';
	me.url = `/api2/extjs/access/tfa/${tfa_id}`;
	me.method = 'PUT';
	me.autoLoad = true;
	return {};
    },

    initComponent: function() {
	let me = this;
	me.callParent();

	if (Proxmox.UserName === 'root@pam') {
	    me.lookup('password').setVisible(false);
	    me.lookup('password').setDisabled(true);
	}

	let userid = me.tfa_id.split('/')[0];
	me.lookup('userid').setValue(userid);
    },

    items: [
	{
	    xtype: 'displayfield',
	    reference: 'userid',
	    editable: false,
	    fieldLabel: gettext('User'),
	    editConfig: {
		xtype: 'pbsUserSelector',
		allowBlank: false,
	    },
	    value: Proxmox.UserName,
	},
	{
	    xtype: 'proxmoxtextfield',
	    name: 'description',
	    allowBlank: false,
	    fieldLabel: gettext('Description'),
	},
	{
	    xtype: 'proxmoxcheckbox',
	    fieldLabel: gettext('Enabled'),
	    name: 'enable',
	    uncheckedValue: 0,
	    defaultValue: 1,
	    checked: true,
	},
	{
	    xtype: 'textfield',
	    inputType: 'password',
	    fieldLabel: gettext('Password'),
	    minLength: 5,
	    reference: 'password',
	    name: 'password',
	    allowBlank: false,
	    validateBlank: true,
	    emptyText: gettext('verify current password'),
	},
    ],

    getValues: function() {
	var me = this;

	var values = me.callParent(arguments);

	delete values.userid;

	return values;
    },
});
