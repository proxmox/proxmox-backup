Ext.define('PBS.form.NotifyType', {
    extend: 'Proxmox.form.KVComboBox',
    alias: 'widget.pbsNotifyType',

    comboItems: [
	['__default__', gettext('Default (Always)')],
	['always', gettext('Always')],
	['error', gettext('Errors')],
	['never', gettext('Never')],
    ],
});

Ext.define('PBS.form.NotifyErrorDefaultType', {
    extend: 'Proxmox.form.KVComboBox',
    alias: 'widget.pbsNotifyErrorDefaultType',

    comboItems: [
	['__default__', gettext('Default (Errors)')],
	['always', gettext('Always')],
	['error', gettext('Errors')],
	['never', gettext('Never')],
    ],
});

Ext.define('PBS.window.NotifyOptions', {
    extend: 'Proxmox.window.Edit',
    xtype: 'pbsNotifyOptionEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'maintenance_notification',

    user: undefined,
    tokenname: undefined,

    isAdd: false,
    isCreate: false,

    subject: gettext('Datastore Options'),
    // hack to avoid that the trigger of the combogrid fields open on window show
    defaultFocus: 'proxmoxHelpButton',

    width: 450,
    fieldDefaults: {
	labelWidth: 120,
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let notify = {};
	    for (const k of ['verify', 'sync', 'gc', 'prune']) {
		notify[k] = values[k];
		delete values[k];
	    }
	    values.notify = PBS.Utils.printPropertyString(notify);

	    PBS.Utils.delete_if_default(values, 'notify', '');
	    PBS.Utils.delete_if_default(values, 'notify-user', '');

	    return values;
	},
	items: [
	    {
		xtype: 'pmxUserSelector',
		name: 'notify-user',
		fieldLabel: gettext('Notify User'),
		emptyText: 'root@pam',
		value: null,
		allowBlank: true,
		renderer: Ext.String.htmlEncode,
		deleteEmpty: true,
	    },
	    {
		xtype: 'pbsNotifyType',
		name: 'verify',
		fieldLabel: gettext('Verification Jobs'),
		value: '__default__',
		deleteEmpty: false,
	    },
	    {
		xtype: 'pbsNotifyType',
		name: 'sync',
		fieldLabel: gettext('Sync Jobs'),
		value: '__default__',
		deleteEmpty: false,
	    },
	    {
		xtype: 'pbsNotifyErrorDefaultType',
		name: 'prune',
		fieldLabel: gettext('Prune Jobs'),
		value: '__default__',
		deleteEmpty: false,
	    },
	    {
		xtype: 'pbsNotifyType',
		name: 'gc',
		fieldLabel: gettext('Garbage Collection'),
		value: '__default__',
		deleteEmpty: false,
	    },
	],
    },
    setValues: function(values) {
	let me = this;

	// we only handle a reduced set of options here
	let options = {
	    'notify-user': values['notify-user'],
	    'verify-new': values['verify-new'],
	};

	let notify = {};
	if (values.notify) {
	    notify = PBS.Utils.parsePropertyString(values.notify);
	}
	Object.assign(options, notify);

	me.callParent([options]);
    },
});
