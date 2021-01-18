Ext.define('PBS.window.AddTfaRecovery', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsAddTfaRecovery',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_mgmt',
    isCreate: true,
    isAdd: true,
    subject: gettext('TFA recovery keys'),
    width: 512,
    method: 'POST',

    fixedUser: false,

    url: '/api2/extjs/access/tfa',
    submitUrl: function(url, values) {
	let userid = values.userid;
	delete values.userid;
	return `${url}/${userid}`;
    },

    apiCallDone: function(success, response) {
	if (!success) {
	    return;
	}

	let values = response.result.data.recovery.join("\n");
	Ext.create('PBS.window.TfaRecoveryShow', {
	    autoShow: true,
	    values,
	});
    },

    viewModel: {
	data: {
	    has_entry: false,
	    userid: Proxmox.UserName,
	},
	formulas: {
	    passwordConfirmText: (get) => {
		let id = get('userid');
		return Ext.String.format(gettext("Confirm password of '{0}'"), id);
	    },
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',
	hasEntry: async function(userid) {
	    let me = this;
	    let view = me.getView();

	    try {
		await PBS.Async.api2({
		    url: `${view.url}/${userid}/recovery`,
		    method: 'GET',
		});
		return true;
	    } catch (_ex) {
		return false;
	    }
	},

	init: function(view) {
	    this.onUseridChange(null, Proxmox.UserName);
	},

	onUseridChange: async function(field, userid) {
	    let me = this;
	    let vm = me.getViewModel();

	    me.userid = userid;
	    vm.set('userid', userid);

	    let has_entry = await me.hasEntry(userid);
	    vm.set('has_entry', has_entry);
	},
    },

    items: [
	{
	    xtype: 'pmxDisplayEditField',
	    name: 'userid',
	    cbind: {
		editable: (get) => !get('fixedUser'),
	    },
	    fieldLabel: gettext('User'),
	    editConfig: {
		xtype: 'pbsUserSelector',
		allowBlank: false,
		validator: function(_value) {
		    return !this.up('window').getViewModel().get('has_entry');
		},
	    },
	    renderer: Ext.String.htmlEncode,
	    value: Proxmox.UserName,
	    listeners: {
		change: 'onUseridChange',
	    },
	},
	{
	    xtype: 'hiddenfield',
	    name: 'type',
	    value: 'recovery',
	},
	{
	    xtype: 'displayfield',
	    bind: {
		hidden: '{!has_entry}',
	    },
	    hidden: true,
	    userCls: 'pmx-hint',
	    value: gettext('User already has recovery keys.'),
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
	    hidden: Proxmox.UserName === 'root@pam',
	    disabled: Proxmox.UserName === 'root@pam',
	    bind: {
		emptyText: '{passwordConfirmText}',
	    },
	},
    ],
});

Ext.define('PBS.window.TfaRecoveryShow', {
    extend: 'Ext.window.Window',
    alias: ['widget.pbsTfaRecoveryShow'],
    mixins: ['Proxmox.Mixin.CBind'],

    width: 600,
    modal: true,
    resizable: false,
    title: gettext('Recovery Keys'),

    items: [
	{
	    xtype: 'form',
	    layout: 'anchor',
	    bodyPadding: 10,
	    border: false,
	    fieldDefaults: {
		anchor: '100%',
            },
	    items: [
		{
		    xtype: 'textarea',
		    editable: false,
		    inputId: 'token-secret-value',
		    cbind: {
			value: '{values}',
		    },
		    fieldStyle: {
			'fontFamily': 'monospace',
		    },
		    height: '160px',
		},
		{
		    xtype: 'displayfield',
		    border: false,
		    padding: '5 0 0 0',
		    userCls: 'pmx-hint',
		    value: gettext('Please record recovery keys - they will only be displayed now'),
		},
	    ],
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
