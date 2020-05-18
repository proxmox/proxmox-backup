Ext.define('PBS.window.UserEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsUserEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    isAdd: true,

    subject: gettext('User'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	var me = this;

	var userid = initialConfig.userid;
	var baseurl = '/api2/extjs/access/users';

	me.isCreate = !userid;
	me.url = userid ? baseurl + '/' + userid : baseurl;
	me.method = userid ? 'PUT' : 'POST';
	me.autoLoad = !!userid;

	return {
	    useridXType: userid ? 'displayfield' : 'textfield',
	};
    },

    items: {
	xtype: 'inputpanel',
	column1: [
	    {
		xtype: 'pmxDisplayEditField',
		name: 'userid',
		fieldLabel: gettext('User name'),
		renderer: Ext.htmlEncode,
		allowBlank: false,
		minLength: 4,
		cbind: {
		    editable: '{isCreate}',
		},
	    },
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
		cbind: {
		    hidden: '{!isCreate}',
		    disabled: '{!isCreate}',
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
		cbind: {
		    hidden: '{!isCreate}',
		    disabled: '{!isCreate}',
		},
	    },
	    {
                xtype: 'datefield',
                name: 'expire',
		emptyText: Proxmox.Utils.neverText,
		format: 'Y-m-d',
		submitFormat: 'U',
                fieldLabel: gettext('Expire'),
            },
	    {
		xtype: 'proxmoxcheckbox',
		fieldLabel: gettext('Enabled'),
		name: 'enable',
		uncheckedValue: 0,
		defaultValue: 1,
		checked: true,
	    },
	],

	column2: [
	    {
		xtype: 'proxmoxtextfield',
		name: 'firstname',
		fieldLabel: gettext('First Name'),
	    },
	    {
		xtype: 'proxmoxtextfield',
		name: 'lastname',
		fieldLabel: gettext('Last Name'),
	    },
	    {
		xtype: 'proxmoxtextfield',
		name: 'email',
		fieldLabel: gettext('E-Mail'),
		vtype: 'proxmoxMail',
	    },
	],

	columnB: [
	    {
		xtype: 'proxmoxtextfield',
		name: 'comment',
		fieldLabel: gettext('Comment'),
	    },
	],
    },

    getValues: function(dirtyOnly) {
	var me = this;

	var values = me.callParent(arguments);

	// hack: ExtJS datefield does not submit 0, so we need to set that
	if (!values.expire) {
	    values.expire = 0;
	}

	if (me.isCreate) {
	    values.userid = values.userid + '@pbs';
	}

	delete values.username;

	if (!values.password) {
	    delete values.password;
	}

	return values;
    },

    setValues: function(values) {
	var me = this;

	if (Ext.isDefined(values.expire)) {
	    if (values.expire) {
		values.expire = new Date(values.expire * 1000);
	    } else {
		// display 'never' instead of '1970-01-01'
		values.expire = null;
	    }
	}

	me.callParent([values]);
    },
});
