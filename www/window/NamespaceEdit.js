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
