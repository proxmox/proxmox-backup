Ext.define('PBS.window.TokenEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsTokenEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_tokens',

    user: undefined,
    tokenname: undefined,

    isAdd: true,
    isCreate: false,
    fixedUser: false,

    subject: gettext('API token'),

    fieldDefaults: { labelWidth: 120 },

    items: {
	xtype: 'inputpanel',
	column1: [
	    {
		xtype: 'pmxDisplayEditField',
		cbind: {
		    editable: (get) => get('isCreate') && !get('fixedUser'),
		    value: () => Proxmox.UserName,
		},
		editConfig: {
		    xtype: 'pmxUserSelector',
		    allowBlank: false,
		},
		name: 'user',
		renderer: Ext.String.htmlEncode,
		fieldLabel: gettext('User'),
	    },
	    {
		xtype: 'pmxDisplayEditField',
		cbind: {
		    editable: '{isCreate}',
		},
		name: 'tokenname',
		fieldLabel: gettext('Token Name'),
		minLength: 2,
		allowBlank: false,
	    },
	],

	column2: [
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
	    me.url = '/api2/extjs/access/users/';
	    let uid = encodeURIComponent(values.user);
	    let tid = encodeURIComponent(values.tokenname);
	    delete values.user;
	    delete values.tokenname;

	    me.url += `${uid}/token/${tid}`;
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

    initComponent: function() {
	let me = this;

	me.url = '/api2/extjs/access/users/';

	me.callParent();

	if (me.isCreate) {
	    me.method = 'POST';
	} else {
	    me.method = 'PUT';

	    let uid = encodeURIComponent(me.user);
	    let tid = encodeURIComponent(me.tokenname);

	    me.url += `${uid}/token/${tid}`;
	    me.load({
		success: function(response, options) {
		    let values = response.result.data;
		    values.user = me.user;
		    values.tokenname = me.tokenname;
		    me.setValues(values);
		},
	    });
	}
    },

    apiCallDone: function(success, response, options) {
	let res = response.result.data;
	if (!success || !res || !res.value) {
	    return;
	}

	Ext.create('PBS.window.TokenShow', {
	    autoShow: true,
	    tokenid: res.tokenid,
	    secret: res.value,
	});
    },
});

Ext.define('PBS.window.TokenShow', {
    extend: 'Ext.window.Window',
    alias: ['widget.pbsTokenShow'],
    mixins: ['Proxmox.Mixin.CBind'],

    width: 600,
    modal: true,
    resizable: false,
    title: gettext('Token Secret'),

    items: [
	{
	    xtype: 'container',
	    layout: 'form',
	    bodyPadding: 10,
	    border: false,
	    fieldDefaults: {
		labelWidth: 100,
		anchor: '100%',
            },
	    padding: '0 10 10 10',
	    items: [
		{
		    xtype: 'textfield',
		    fieldLabel: gettext('Token ID'),
		    cbind: {
			value: '{tokenid}',
		    },
		    editable: false,
		},
		{
		    xtype: 'textfield',
		    fieldLabel: gettext('Secret'),
		    inputId: 'token-secret-value',
		    cbind: {
			value: '{secret}',
		    },
		    editable: false,
		},
	    ],
	},
	{
	    xtype: 'component',
	    border: false,
	    padding: '10 10 10 10',
	    userCls: 'pmx-hint',
	    html: gettext('Please record the API token secret - it will only be displayed now'),
	},
    ],
    buttons: [
	{
	    handler: function(b) {
		document.getElementById('token-secret-value').select();
		document.execCommand("copy");
	    },
	    text: gettext('Copy Secret Value'),
	},
    ],
});
