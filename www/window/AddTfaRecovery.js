Ext.define('PBS.window.AddTfaRecovery', {
    extend: 'Ext.window.Window',
    alias: 'widget.pbsAddTfaRecovery',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'user_mgmt',

    modal: true,
    resizable: false,
    title: gettext('Add TFA recovery keys'),
    width: 512,

    fixedUser: false,

    baseurl: '/api2/extjs/access/tfa',

    initComponent: function() {
	let me = this;
	me.callParent();
	Ext.GlobalEvents.fireEvent('proxmoxShowHelp', me.onlineHelp);
    },

    viewModel: {
	data: {
	    has_entry: false,
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',
	control: {
	    '#': {
		show: function() {
		    let me = this;
		    let view = me.getView();

		    if (Proxmox.UserName === 'root@pam') {
			view.lookup('password').setVisible(false);
			view.lookup('password').setDisabled(true);
		    }
		},
	    },
	},

	hasEntry: async function(userid) {
	    let me = this;
	    let view = me.getView();

	    try {
		await PBS.Async.api2({
		    url: `${view.baseurl}/${userid}/recovery`,
		    method: 'GET',
		});
		return true;
	    } catch (_ex) {
		return false;
	    }
	},

	init: function() {
	    this.onUseridChange(null, Proxmox.UserName);
	},

	onUseridChange: async function(_field, userid) {
	    let me = this;

	    me.userid = userid;

	    let has_entry = await me.hasEntry(userid);
	    me.getViewModel().set('has_entry', has_entry);
	},

	onAdd: async function() {
	    let me = this;
	    let view = me.getView();

	    let baseurl = view.baseurl;

	    let userid = me.userid;
	    if (userid === undefined) {
		throw "no userid set";
	    }

	    me.getView().close();

	    try {
		let response = await PBS.Async.api2({
		    url: `${baseurl}/${userid}`,
		    method: 'POST',
		    params: { type: 'recovery' },
		});
		let values = response.result.data.recovery.join("\n");
		Ext.create('PBS.window.TfaRecoveryShow', {
		    autoShow: true,
		    values,
		});
	    } catch (ex) {
		Ext.Msg.alert(gettext('Error'), ex);
	    }
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
	    },
	    renderer: Ext.String.htmlEncode,
	    value: Proxmox.UserName,
	    listeners: {
		change: 'onUseridChange',
	    },
	},
	{
	    xtype: 'displayfield',
	    bind: {
		hidden: '{!has_entry}',
	    },
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
	    padding: '0 0 5 5',
	    emptyText: gettext('verify current password'),
	},
    ],

    buttons: [
	{
	    xtype: 'proxmoxHelpButton',
	},
	'->',
	{
	    xtype: 'button',
	    text: gettext('Add'),
	    handler: 'onAdd',
	    bind: {
		disabled: '{has_entry}',
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
	    ],
	},
	{
	    xtype: 'component',
	    border: false,
	    padding: '10 10 10 10',
	    userCls: 'pmx-hint',
	    html: gettext('Please record recovery keys - they will only be displayed now'),
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
