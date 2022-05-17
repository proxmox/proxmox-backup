Ext.define('PBS.window.NamespaceEdit', {
    extend: 'Proxmox.window.Edit',
    xtype: 'pbsNamespaceEdit', // for now rather "NamespaceAdd"
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'storage-namespaces',

    isCreate: true,
    subject: gettext('Namespace'),
    // avoid that the trigger of the combogrid fields open on window show
    defaultFocus: 'proxmoxtextfield[name=name]',

    cbind: {
	url: '/api2/extjs/admin/datastore/{datastore}/namespace',
    },
    method: 'POST',

    width: 450,
    fieldDefaults: {
	labelWidth: 120,
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    if (values.parent === '') {
		delete values.parent;
	    }
	    return values;
	},
	items: [
	    {
		xtype: 'pbsNamespaceSelector',
		name: 'parent',
		fieldLabel: gettext('Parent Namespace'),
		cbind: {
		    value: '{namespace}',
		    datastore: '{datastore}',
		},
	    },
	    {
		xtype: 'proxmoxtextfield',
		name: 'name',
		fieldLabel: gettext('Namespace Name'),
		value: '',
		allowBlank: false,
		maxLength: 31,
		regex: PBS.Utils.SAFE_ID_RE,
		regexText: gettext("Only alpha numerical, '_' and '-' (if not at start) allowed"),
	    },
	],
    },
});

Ext.define('PBS.window.NamespaceDelete', {
    extend: 'Proxmox.window.SafeDestroy',
    xtype: 'pbsNamespaceDelete',
    mixins: ['Proxmox.Mixin.CBind'],

    viewModel: {},

    autoShow: true,
    taskName: 'delete-namespace',

    cbind: {
	url: '/api2/extjs/admin/datastore/{datastore}/namespace',
    },
    additionalItems: [
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'delete-groups',
	    reference: 'rmGroups',
	    boxLabel: gettext('Delete all Backup Groups'),
	    value: false,
	    listeners: {
		change: function(field, value) {
		    let win = field.up('proxmoxSafeDestroy');
		    if (value) {
			win.params['delete-groups'] = value;
		    } else {
			delete win.params['delete-groups'];
		    }
		},
	    },
	},
	{
	    xtype: 'box',
	    padding: '5 0 0 0',
	    html: `<span class="pmx-hint">${gettext('Note')}</span>: `
	      + gettext('This will permanently remove all backups from the current namespace and all namespaces below it!'),
	    bind: {
		hidden: '{!rmGroups.checked}',
	    },
	},
    ],

    initComponent: function() {
	let me = this;
	me.title = Ext.String.format(gettext("Destroy Namespace '{0}'"), me.namespace);
	me.params = { ns: me.namespace };

	me.callParent();
    },
});
