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

	let values = response
	    .result
	    .data
	    .recovery
	    .map((v, i) => `${i}: ${v}`)
	    .join("\n");
	Ext.create('PBS.window.TfaRecoveryShow', {
	    autoShow: true,
	    userid: this.getViewModel().get('userid'),
	    values,
	});
    },

    viewModel: {
	data: {
	    has_entry: false,
	    userid: null,
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',
	hasEntry: async function(userid) {
	    let me = this;
	    let view = me.getView();

	    try {
		await Proxmox.Async.api2({
		    url: `${view.url}/${userid}/recovery`,
		    method: 'GET',
		});
		return true;
	    } catch (_response) {
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
		value: () => Proxmox.UserName,
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
	    name: 'password',
	    reference: 'password',
	    fieldLabel: gettext('Verify Password'),
	    inputType: 'password',
	    minLength: 5,
	    allowBlank: false,
	    validateBlank: true,
	    cbind: {
		hidden: () => Proxmox.UserName === 'root@pam',
		disabled: () => Proxmox.UserName === 'root@pam',
		emptyText: () =>
		    Ext.String.format(gettext("Confirm your ({0}) password"), Proxmox.UserName),
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
    onEsc: Ext.emptyFn,

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
	    iconCls: 'fa fa-clipboard',
	    text: gettext('Copy Recovery Keys'),
	},
	{
	    handler: function(b) {
		let win = this.up('window');
		win.paperkeys(win.values, win.userid);
	    },
	    iconCls: 'fa fa-print',
	    text: gettext('Print Recovery Keys'),
	},
    ],
    paperkeys: function(keyString, userid) {
	let me = this;

	let printFrame = document.createElement("iframe");
	Object.assign(printFrame.style, {
	    position: "fixed",
	    right: "0",
	    bottom: "0",
	    width: "0",
	    height: "0",
	    border: "0",
	});
	const host = document.location.host;
	const title = document.title;
	const html = `<html><head><script>
	    window.addEventListener('DOMContentLoaded', (ev) => window.print());
	</script><style>@media print and (max-height: 150mm) {
	  h4, p { margin: 0; font-size: 1em; }
	}</style></head><body style="padding: 5px;">
	<h4>Recovery Keys for '${userid}' - ${title} (${host})</h4>
<p style="font-size:1.5em;line-height:1.5em;font-family:monospace;
   white-space:pre-wrap;overflow-wrap:break-word;">
${keyString}
</p>
	</body></html>`;

	printFrame.src = "data:text/html;base64," + btoa(html);
	document.body.appendChild(printFrame);
    },
});
