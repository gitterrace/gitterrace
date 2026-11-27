// The landing page's one script, inline and hashed into the CSP like the theme button's.
// Whole-line comments stay in this file. Without it the page still works: the command is
// plain text, and the window turns by touch, by its links and by the arrow keys.
(function(){var d=document,r=d.getElementById('reel'),b=d.querySelector('.copy');
// Copy: the label reports what happened. A refused clipboard selects the command instead.
if(b){var t=b.textContent;b.addEventListener('click',function(){
function say(s,w){b.dataset.state=s;b.textContent=w;setTimeout(function(){
delete b.dataset.state;b.textContent=t},2000)}
try{navigator.clipboard.writeText(b.dataset.copy).then(function(){say('copied','Copied')},
sel)}catch(e){sel()}
function sel(){getSelection().selectAllChildren(d.querySelector('.inst b'));
say('selected','Selected')}})}
if(!r)return;
// The window: the links and the two buttons scroll the row, and the row's own position
// says which link is current, so touch, keys and buttons can never disagree.
var m=d.querySelectorAll('.marks a'),n=m.length,u=d.querySelector('.turn');
function at(){return Math.round(r.scrollLeft/r.clientWidth)}
function go(i){r.scrollTo({left:(i+n)%n*r.clientWidth})}
function mark(){m.forEach(function(a,i){if(i===at())a.setAttribute('aria-current','true');
else a.removeAttribute('aria-current')})}
m.forEach(function(a,i){a.addEventListener('click',function(e){e.preventDefault();go(i)})});
u.hidden=false;u.firstElementChild.addEventListener('click',function(){go(at()-1)});
u.lastElementChild.addEventListener('click',function(){go(at()+1)});
r.addEventListener('keydown',function(e){var k={ArrowLeft:-1,ArrowRight:1}[e.key];
if(k){e.preventDefault();go(at()+k)}});
r.addEventListener('scroll',mark,{passive:true});mark()})()
