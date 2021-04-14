Ext.define('PBS.TapeManagement.EraseWindow', {
    extend: 'Proxmox.window.Edit',
    mixins: ['Proxmox.Mixin.CBind'],


    changer: undefined,
    label: undefined,

    cbindData: function(config) {
	let me = this;
	return {
	    singleDrive: me.singleDrive,
	    hasSingleDrive: !!me.singleDrive,
	    warning: Ext.String.format(gettext("Are you sure you want to format tape '{0}' ?"), me.label),
	};
    },

    title: gettext('Format/Erase'),
    url: `/api2/extjs/tape/drive`,
    showProgress: true,
    submitUrl: function(url, values) {
	let drive = values.drive;
	delete values.drive;
	return `${url}/${drive}/format-media`;
    },

    layout: 'hbox',
    width: 400,
    method: 'POST',
    isCreate: true,
    submitText: gettext('Ok'),
    items: [
	{
	    xtype: 'container',
	    padding: 0,
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },
	    items: [
		{
		    xtype: 'component',
		    cls: [Ext.baseCSSPrefix + 'message-box-icon',
			Ext.baseCSSPrefix + 'message-box-warning',
			Ext.baseCSSPrefix + 'dlg-icon'],
		},
		{
		    xtype: 'container',
		    flex: 1,
		    items: [
			{
			    xtype: 'displayfield',
			    cbind: {
				value: '{warning}',
			    },
			},
			{
			    xtype: 'displayfield',
			    cls: 'pmx-hint',
			    value: gettext('Make sure to insert the tape into the selected drive.'),
			    cbind: {
				hidden: '{changer}',
			    },
			},
			{
			    xtype: 'hidden',
			    name: 'label-text',
			    cbind: {
				value: '{label}',
			    },
			},
			{
			    xtype: 'hidden',
			    name: 'drive',
			    cbind: {
				disabled: '{!hasSingleDrive}',
				value: '{singleDrive}',
			    },
			},
			{
			    xtype: 'pbsDriveSelector',
			    fieldLabel: gettext('Drive'),
			    name: 'drive',
			    cbind: {
				changer: '{changer}',
				disabled: '{hasSingleDrive}',
				hidden: '{hasSingleDrive}',
			    },
			},
		    ],
		},
	    ],
	},
    ],
});
