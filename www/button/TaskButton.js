Ext.define('PBS.TaskButton', {
    extend: 'Ext.button.Button',
    alias: 'widget.pbsTaskButton',

    config: {
	badgeText: '0',
	badgeCls: '',
    },

    iconCls: 'fa fa-list-alt',
    userCls: 'pmx-has-badge',
    text: gettext('Tasks'),

    setText: function(value) {
	let me = this;
	me.realText = value;
	let badgeText = me.getBadgeText();
	let badgeCls = me.getBadgeCls();
	let text = `${value} <span class="pmx-button-badge ${badgeCls}">${badgeText}</span>`;
	return me.callParent([text]);
    },

    getText: function() {
	let me = this;
	return me.realText;
    },

    setBadgeText: function(value) {
	let me = this;
	me.badgeText = value.toString();
	return me.setText(me.getText());
    },

    setBadgeCls: function(value) {
	let me = this;
	let res = me.callParent([value]);
	let badgeText = me.getBadgeText();
	me.setBadgeText(badgeText);
	return res;
    },

    handler: function() {
	let me = this;
	if (me.grid.isVisible()) {
	    me.grid.setVisible(false);
	} else {
	    me.grid.showBy(me, 'tr-br');
	}
    },

    initComponent: function() {
	let me = this;

	me.grid = Ext.create({
	    xtype: 'pbsRunningTasks',
	    title: '',
	    hideHeaders: false,
	    floating: true,

	    width: 600,

	    bbar: [
		'->',
		{
		    xtype: 'button',
		    text: gettext('Show All Tasks'),
		    handler: function() {
			var mainview = me.up('mainview');
			mainview.getController().redirectTo('pbsServerAdministration:tasks');
			me.grid.hide();
		    },
		},
	    ],

	    listeners: {
		'taskopened': function() {
		    me.grid.hide();
		},
	    },
	});
	me.callParent();
	me.mon(me.grid.getStore().rstore, 'load', function(store, records, success) {
	    if (!success) return;

	    let count = records.length;
	    let text = count > 99 ? '99+' : count.toString();
	    let cls = count > 0 ? 'active': '';
	    me.setBadgeText(text);
	    me.setBadgeCls(cls);
	});
    },
});
