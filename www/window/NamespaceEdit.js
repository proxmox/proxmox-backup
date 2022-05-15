Ext.define('PBS.window.NamespaceEdit', {
    extend: 'Proxmox.window.Edit',
    xtype: 'pbsNamespaceEdit', // for now rather "NamespaceAdd"
    mixins: ['Proxmox.Mixin.CBind'],

    //onlineHelp: 'namespaces', // TODO

    isCreate: true,
    subject: gettext('Namespace'),
    // avoid that the trigger of the combogrid fields open on window show
    defaultFocus: 'proxmoxHelpButton',

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
    extend: 'Proxmox.window.Edit',
    xtype: 'pbsNamespaceDelete',
    mixins: ['Proxmox.Mixin.CBind'],

    //onlineHelp: 'namespaces', // TODO

    viewModel: {},

    isRemove: true,
    isCreate: true, // because edit window is, well, a bit stupid..
    title: gettext('Destroy Namespace'),
    // avoid that the trigger of the combogrid fields open on window show
    defaultFocus: 'proxmoxHelpButton',

    cbind: {
	url: '/api2/extjs/admin/datastore/{datastore}/namespace',
    },
    method: 'DELETE',

    width: 450,

    items: {
	xtype: 'inputpanel',
	items: [
	    {
		xtype: 'displayfield',
		name: 'ns',
		fieldLabel: gettext('Namespace'),
		cbind: {
		    value: '{namespace}',
		    datastore: '{datastore}',
		},
		submitValue: true,
	    },
	    {
		xtype: 'proxmoxcheckbox',
		name: 'delete-groups',
		reference: 'rmGroups',
		boxLabel: gettext('Delete all Backup Groups'),
		value: false,
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
    },
});
