Ext.define('PBS.window.VerifyAll', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsVerifyAll',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'maintenance_verification',

    method: 'POST',
    cbind: {
	title: `Verify Datastore '{datastore}'`,
	url: `/admin/datastore/{datastore}/verify`,
    },

    submitText: gettext('Verify'),
    isCreate: true,
    showTaskViewer: true,
    showReset: false,
    defaultFocus: 'submitbutton',
    width: 450,
    items: [
	{
	    xtype: 'inputpanel',
	    viewModel: {
		data: { ignoreVerified: true },
	    },
	    onGetValues: values => {
		if (!values.ns || values.ns === '') {
		    delete values.ns;
		}
		return values;
	    },
	    items: [
		{
		    xtype: 'pbsNamespaceSelector',
		    name: 'ns',
		    fieldLabel: gettext('Namespace'),
		    cbind: {
			datastore: '{datastore}',
			value: '{namespace}',
		    },
		},
		{
		    xtype: 'pbsNamespaceMaxDepth',
		    name: 'max-depth',
		    deleteEmpty: false,
		},
		{
		    xtype: 'fieldcontainer',
		    layout: 'hbox',
		    fieldLabel: gettext('Skip Verified'),
		    items: [
			{
			    xtype: 'proxmoxcheckbox',
			    name: 'ignore-verified',
			    uncheckedValue: false,
			    value: true,
			    bind: {
				value: '{ignoreVerified}',
			    },
			},
			{
			    xtype: 'pbsVerifyOutdatedAfter',
			    name: 'outdated-after',
			    fieldLabel: gettext('Re-Verify After'),
			    padding: '0 0 0 5',
			    bind: {
				disabled: '{!ignoreVerified}',
			    },
			    flex: 1,
			},
			{
			    xtype: 'displayfield',
			    name: 'unit',
			    submitValue: false,
			    padding: '0 0 0 5',
			    value: gettext('days'),
			    bind: {
				disabled: '{!ignoreVerified}',
			    },
			},
		    ],
		},
	    ],
	},
    ],
});
