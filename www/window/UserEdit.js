Ext.define('PBS.window.UserEditViewModel', {
    extend: 'Ext.app.ViewModel',
    alias: 'viewmodel.pbsUserEdit',

    data: {
	realm: 'pbs',
    },

    formulas: {
	maySetPassword: function(get) {
	    let realm = get('realm');

	    let view = this.getView();
	    let realmStore = view.down('pmxRealmComboBox').getStore();
	    if (realmStore.isLoaded()) {
		let rec = realmStore.findRecord('realm', realm, 0, false, true, true);
		return Proxmox.Schema.authDomains[rec.data.type]?.pwchange && view.isCreate;
	    } else {
		return view.isCreate;
	    }
	},
    },
});

Ext.define('PBS.window.UserEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsUserEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_mgmt',

    userid: undefined,

    isAdd: true,

    subject: gettext('User'),

    fieldDefaults: { labelWidth: 120 },

    viewModel: {
	type: 'pbsUserEdit',
    },

    cbindData: function(initialConfig) {
	var me = this;

	var userid = initialConfig.userid;
	var baseurl = '/api2/extjs/access/users';

	me.isCreate = !userid;
	me.url = userid ? baseurl + '/' + encodeURIComponent(userid) : baseurl;
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
		xtype: 'pmxRealmComboBox',
		name: 'realm',
		fieldLabel: gettext('Realm'),
		allowBlank: false,
		matchFieldWidth: false,
		listConfig: { width: 300 },
		bind: {
		    value: '{realm}',
		},
		cbind: {
		    hidden: '{!isCreate}',
		    disabled: '{!isCreate}',
		},
		submitValue: true,
		storeFilter: rec => rec.data?.type !== 'pam',
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
		bind: {
		    disabled: '{!maySetPassword}',
		    hidden: '{!maySetPassword}',
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
		bind: {
		    disabled: '{!maySetPassword}',
		    hidden: '{!maySetPassword}',
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
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	    {
		xtype: 'proxmoxtextfield',
		name: 'lastname',
		fieldLabel: gettext('Last Name'),
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	    {
		xtype: 'proxmoxtextfield',
		name: 'email',
		fieldLabel: gettext('E-Mail'),
		vtype: 'proxmoxMail',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],

	columnB: [
	    {
		xtype: 'proxmoxtextfield',
		name: 'comment',
		fieldLabel: gettext('Comment'),
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
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
	    values.userid = values.userid + '@' + values.realm;
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
